//! Axum daemon: routes, live-reload WebSocket, filesystem watcher.

use crate::runtime::{self, DaemonInfo};
use crate::terminal_auth::{self, HasTerminalAuth, TerminalAuth};
use crate::views;
use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Form, Path, Query, State,
    },
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use mdview_core::indexer::now_rfc3339;
use mdview_core::render::theme_css;
use mdview_core::Engine;
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub reload_tx: broadcast::Sender<String>,
    pub highlight_css: Arc<String>,
    /// Overrides `mdview_core::config::data_dir()` for the settings routes
    /// (`/settings`, `/api/config`) so a route-level test can point config I/O
    /// at a temp dir instead of the developer's real `~/.mdview`. `None` in
    /// production — those routes then resolve exactly where they always did.
    pub config_data_dir: Option<PathBuf>,
    /// The terminal auth mechanism (agent-terminal-3): token file + live
    /// session set. Constructed once per `AppState` so the in-memory session
    /// set survives across requests; a route test that overrides
    /// `config_data_dir` must construct this with the same directory
    /// (`TerminalAuth::new(Some(dir))`), or it will resolve the token at the
    /// real `~/.mdview` instead of the test's scratch dir.
    pub terminal_auth: TerminalAuth,
}

impl HasTerminalAuth for AppState {
    fn terminal_auth(&self) -> &TerminalAuth {
        &self.terminal_auth
    }
}

/// Start the daemon: watcher + HTTP server. Blocks until shutdown.
pub async fn serve() -> Result<()> {
    let engine = Arc::new(runtime::build_engine()?);
    let (reload_tx, _) = broadcast::channel::<String>(32);
    let highlight_css = Arc::new(build_highlight_css(&engine));

    let state = AppState {
        engine: engine.clone(),
        reload_tx: reload_tx.clone(),
        highlight_css,
        config_data_dir: None,
        terminal_auth: TerminalAuth::new(None),
    };

    // Filesystem watcher (kept alive for the process lifetime).
    let _watch = crate::watch::spawn_watchers(engine.clone(), reload_tx.clone())?;

    // Bind with port auto-increment (PRD §10 / mdserve pattern).
    let cfg = &engine.config.server;
    let (listener, addr) = bind_with_retry(&cfg.host, cfg.port).await?;

    runtime::write_lock(&DaemonInfo {
        pid: std::process::id(),
        host: cfg.host.clone(),
        port: addr.port(),
        started_at: now_rfc3339(),
    })?;
    tracing::info!("mdview serving on http://{addr}");
    // A wildcard bind (`0.0.0.0`) makes `http://0.0.0.0:PORT` a dead link, so
    // list every address that actually reaches this server — one per LAN
    // interface (loopback when none) or the configured hostname override.
    let urls = runtime::display_urls_for(&cfg.host, addr.port());
    if urls.len() == 1 {
        println!("mdview serving on {}", urls[0]);
    } else {
        println!("mdview serving on:");
        for url in &urls {
            println!("  {url}");
        }
    }
    if !is_loopback_host(&cfg.host) {
        eprintln!(
            "warning: mdview is bound to a non-loopback address ({}) and has NO \
             authentication — anyone who can reach this port can read every \
             indexed file and each project's filesystem path. Bind 127.0.0.1 \
             unless you intend LAN exposure.",
            cfg.host
        );
    }

    let app = router(state);
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    runtime::remove_lock();
    result?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/health", get(health))
        .route("/api/status", get(status))
        .route("/api/projects", get(api_projects))
        .route("/settings", get(settings_page_handler))
        .route("/api/config", get(api_config).post(update_config))
        .route("/settings/terminal/token", post(rotate_terminal_token))
        .route("/api/terminal-config", post(update_terminal_config))
        .route("/api/projects/:id/unregister", post(unregister_project))
        .route("/static/app.css", get(css_asset))
        .route("/static/app.js", get(js_asset))
        .route("/static/mermaid.min.js", get(mermaid_asset))
        .route("/highlight.css", get(highlight_asset))
        .route("/ws", get(ws_handler))
        .route("/p/:id/", get(project_home))
        .route("/p/:id/_search", get(search_page))
        .route("/p/:id/_jump", get(jump_search))
        .route("/p/:id/_bee", get(bee_board))
        .route("/p/:id/_bee/cell/:cell_id", get(bee_cell_detail))
        .route("/p/:id/_bee/feature/:feature", get(bee_feature_detail))
        .route("/p/:id/*path", get(project_path))
        .with_state(state)
}

async fn index_page(State(st): State<AppState>) -> Response {
    match st.engine.list_projects() {
        Ok(projects) => {
            let with_counts: Vec<_> = projects
                .into_iter()
                .map(|p| {
                    let c = st.engine.file_count(&p.id).unwrap_or(0);
                    (p, c)
                })
                .collect();
            Html(views::project_list_page(&with_counts)).into_response()
        }
        Err(e) => internal_error(&e.to_string()),
    }
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "app": "mdview", "version": env!("CARGO_PKG_VERSION") }))
}

async fn status(State(st): State<AppState>) -> impl IntoResponse {
    let projects = st.engine.list_projects().unwrap_or_default();
    let files: usize = st.engine.store.total_file_count().unwrap_or(0);
    Json(json!({
        "running": true,
        "app": "mdview",
        "version": env!("CARGO_PKG_VERSION"),
        "project_count": projects.len(),
        "indexed_file_count": files,
    }))
}

async fn api_projects(State(st): State<AppState>) -> impl IntoResponse {
    let projects = st.engine.list_projects().unwrap_or_default();
    let arr: Vec<_> = projects
        .into_iter()
        .map(|p| {
            let count = st.engine.file_count(&p.id).unwrap_or(0);
            project_summary_json(&p.id, &p.name, count)
        })
        .collect();
    Json(json!({ "projects": arr }))
}

/// One project's public API summary. Deliberately omits the absolute
/// `root_path`: the server has no authentication, so exposing each project's
/// filesystem layout over `/api/projects` leaks it to anyone who can reach the
/// port (see the non-loopback bind warning in `serve`).
fn project_summary_json(id: &str, name: &str, file_count: usize) -> serde_json::Value {
    json!({
        "id": id,
        "name": name,
        "file_count": file_count,
        "url": format!("/p/{id}/"),
    })
}

async fn api_config(State(st): State<AppState>) -> impl IntoResponse {
    // Read through the same injectable path settings_page_handler and
    // update_config use, rather than the engine's startup-cached config, so
    // all three agree with each other and a route test never touches the
    // real ~/.mdview.
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    Json(json!(cfg))
}

#[derive(serde::Deserialize)]
struct SavedFlag {
    saved: Option<String>,
}

async fn settings_page_handler(State(st): State<AppState>, Query(flag): Query<SavedFlag>) -> Response {
    // Read fresh from disk so the form reflects the last save (the running daemon
    // still uses its startup config until restarted — noted in the UI).
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    let token_view = current_token_view(&st);
    Html(views::settings_page(&cfg, flag.saved.is_some(), token_view)).into_response()
}

/// The token state `settings_page` renders on every ordinary render — masked
/// to the last four characters, or "never generated". This is the *only*
/// path GET /settings uses; the full value is rendered exclusively from the
/// direct response of `rotate_terminal_token`, never reconstructed here (P2).
fn current_token_view(st: &AppState) -> views::TerminalTokenView {
    match st.terminal_auth.masked() {
        Some(masked) => views::TerminalTokenView::Masked(masked),
        None => views::TerminalTokenView::NotGenerated,
    }
}

/// POST /settings/terminal/token — generate (or rotate) the terminal token,
/// per D10. Deliberately part of the ungated settings surface, not the
/// terminal_auth-gated switch endpoint below: D4 gates the terminal routes,
/// not settings, and CONTEXT.md's Known Risk is discharged by P2 (reveal
/// once, mask forever after) rather than by adding a second auth layer here.
///
/// The response that performs the rotation is the one place the full token
/// is ever rendered (P2) — every later `GET /settings` shows only its last
/// four characters. Because the browser making this request has just
/// demonstrated it can reach the (already-unauthenticated) settings surface,
/// this response also mints a terminal session and sets its cookie, so the
/// same browser can immediately reach the gated switches below without a
/// separate login step.
async fn rotate_terminal_token(State(st): State<AppState>) -> Response {
    match st.terminal_auth.rotate() {
        Ok(full_token) => {
            let session_id = st.terminal_auth.mint_session();
            let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
                st.config_data_dir.as_deref(),
            ));
            let html = views::settings_page(
                &cfg,
                false,
                views::TerminalTokenView::Full(full_token),
            );
            (
                [(header::SET_COOKIE, terminal_auth::session_cookie_header(&session_id))],
                Html(html),
            )
                .into_response()
        }
        Err(e) => internal_error(&e.to_string()),
    }
}

#[derive(serde::Deserialize, Default)]
struct TerminalConfigForm {
    enabled: Option<String>,
    supervisor_enabled: Option<String>,
    notify_enabled: Option<String>,
}

/// POST /api/terminal-config — the D7 switches (terminal enable, herdr
/// supervisor, Telegram notification). Per P3 this is deliberately its own
/// route rather than a field on `SettingsForm`/`update_config`:
/// `POST /api/config` is unauthenticated, so a supervisor switch reachable
/// there would let any LAN visitor make mdview spawn a process. `AuthSession`
/// requires a live terminal session (minted only by `rotate_terminal_token`
/// above or a later login route); on any auth failure the request never
/// reaches this handler at all — `AuthSession`'s extractor short-circuits
/// with the opaque 404 before the switches are read, let alone changed.
async fn update_terminal_config(
    State(st): State<AppState>,
    _session: terminal_auth::AuthSession,
    Form(form): Form<TerminalConfigForm>,
) -> Response {
    let config_path = mdview_core::config::config_path_override(st.config_data_dir.as_deref());
    let mut cfg = mdview_core::Config::load_from(&config_path);
    cfg.terminal.enabled = form.enabled.is_some();
    cfg.terminal.supervisor_enabled = form.supervisor_enabled.is_some();
    cfg.terminal.notify_enabled = form.notify_enabled.is_some();
    let _ = cfg.save_to(&config_path);
    Redirect::to("/settings?saved=1").into_response()
}

#[derive(serde::Deserialize)]
struct SettingsForm {
    port: Option<u16>,
    host: Option<String>,
    hostname: Option<String>,
    open_browser: Option<String>,
    theme: Option<String>,
    syntax_theme: Option<String>,
    debounce_ms: Option<u64>,
    max_file_size_mb: Option<u64>,
    exclude_patterns: Option<String>,
    mcp_enabled: Option<String>,
    mcp_transport: Option<String>,
}

async fn update_config(State(st): State<AppState>, Form(form): Form<SettingsForm>) -> Response {
    let config_path =
        mdview_core::config::config_path_override(st.config_data_dir.as_deref());
    let mut cfg = mdview_core::Config::load_from(&config_path);
    if let Some(p) = form.port {
        if p >= 1 {
            cfg.server.port = p;
        }
    }
    if let Some(h) = form.host {
        let h = h.trim();
        if !h.is_empty() {
            cfg.server.host = h.to_string();
        }
    }
    cfg.server.hostname = normalize_hostname(form.hostname);
    cfg.server.open_browser_on_start = form.open_browser.is_some();
    if let Some(t) = form.theme {
        if ["light", "dark", "system"].contains(&t.as_str()) {
            cfg.renderer.theme = t;
        }
    }
    if let Some(s) = form.syntax_theme {
        let s = s.trim();
        if !s.is_empty() {
            cfg.renderer.syntax_highlight_theme = s.to_string();
        }
    }
    if let Some(d) = form.debounce_ms {
        cfg.indexing.debounce_ms = d;
    }
    if let Some(m) = form.max_file_size_mb {
        if m >= 1 {
            cfg.indexing.max_file_size_mb = m;
        }
    }
    if let Some(ex) = form.exclude_patterns {
        cfg.indexing.exclude_patterns = ex
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
    }
    cfg.mcp.enabled = form.mcp_enabled.is_some();
    if let Some(tr) = form.mcp_transport {
        if ["stdio", "http"].contains(&tr.as_str()) {
            cfg.mcp.transport = tr;
        }
    }
    let _ = cfg.save_to(&config_path);
    Redirect::to("/settings?saved=1").into_response()
}

/// Remove a project from the registry, then return to the project list. This
/// only deletes the registry entry and index — the project's files on disk are
/// untouched, and re-registering re-scans them. NOTE: like every route here it
/// is unauthenticated, so it is reachable by anyone who can reach the server.
async fn unregister_project(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let _ = st.engine.unregister(&id);
    Redirect::to("/").into_response()
}

// The CSS/JS assets are compiled into the binary and change whenever the daemon
// is upgraded, but their URLs never change. Without a cache directive a browser
// (mobile especially) may keep serving a stale copy after an upgrade, so UI
// fixes silently never arrive. `no-cache` forces a revalidation each load; the
// files are tiny and served locally, so the cost is negligible.
const NO_CACHE: (header::HeaderName, &str) = (header::CACHE_CONTROL, "no-cache");

async fn css_asset() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css"), NO_CACHE],
        views::APP_CSS,
    )
}
async fn js_asset() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript"), NO_CACHE],
        views::APP_JS,
    )
}
async fn highlight_asset(State(st): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css"), NO_CACHE],
        st.highlight_css.to_string(),
    )
}
/// Vendored Mermaid bundle. It is large (~3.4 MB) but static across a daemon
/// version, so it may be cached hard — unlike the app's own CSS/JS.
async fn mermaid_asset() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "public, max-age=604800"),
        ],
        views::MERMAID_JS,
    )
}

async fn project_home(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let files = match st.engine.list_files(&id) {
        Ok(files) => files,
        Err(_) => return not_found("project not found"),
    };

    // D3: the bee entry point appears only when the project's root contains a
    // `.bee/` directory — a plain presence check, not a full store read (the
    // page it links to, `/p/:id/_bee`, does the actual reading). A project
    // without `.bee/` falls through unchanged to the redirect/not-found
    // behavior this route always had, so non-bee projects behave exactly as
    // they do today.
    if let Ok(Some(project)) = st.engine.get_project(&id) {
        if is_bee_project(&project) {
            let entry = pick_entry_file(&files).map(|f| f.rel_path.as_str());
            return Html(views::project_home_page(&project, entry)).into_response();
        }
    }

    if files.is_empty() {
        return not_found("project has no markdown files");
    }
    let entry = pick_entry_file(&files).unwrap_or(&files[0]);
    Redirect::to(&format!("/p/{}/{}", id, entry.rel_path)).into_response()
}

/// D3's presence rule: a project shows the bee surface iff its `root_path`
/// contains a `.bee/` directory.
fn is_bee_project(project: &mdview_core::domain::Project) -> bool {
    project.root_path.join(".bee").is_dir()
}

/// `GET /p/:id/_bee` — the read-only cell board (D4). Renders the four D7
/// buckets over the project's live `.bee/cells/`. A project with no `.bee/`
/// gets a clean not-found, never an empty bee page (D3).
async fn bee_board(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    let snapshot = mdview_core::bee::read_snapshot(&project.root_path);
    if !snapshot.present {
        return not_found("this project has no .bee/ store");
    }
    Html(views::bee_board_page(&project, &snapshot)).into_response()
}

/// `GET /p/:id/_bee/cell/:cell_id` — one cell in full (D4): title, action,
/// verify, lane, status, its files/read_first lists, the decisions it
/// cites, its must_haves, and its whole trace. A missing project, an absent
/// `.bee/`, or an unknown cell id each resolve to the same clean not-found
/// (D3), never a blank page.
async fn bee_cell_detail(
    State(st): State<AppState>,
    Path((id, cell_id)): Path<(String, String)>,
) -> Response {
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    if !is_bee_project(&project) {
        return not_found("this project has no .bee/ store");
    }
    match find_cell_full(&project.root_path, &cell_id) {
        Some(cell) => Html(views::bee_cell_page(&project, &cell)).into_response(),
        None => not_found("cell not found"),
    }
}

/// `GET /p/:id/_bee/feature/:feature` — one feature's cells grouped into the
/// same four D7 buckets the board uses, plus its shipped state (D10) and
/// cycle time (D11) when timed. An unknown feature name — none of its cells
/// live in any bucket, and it never shipped — resolves to a clean not-found,
/// same as an unknown cell id.
async fn bee_feature_detail(
    State(st): State<AppState>,
    Path((id, feature)): Path<(String, String)>,
) -> Response {
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    let snapshot = mdview_core::bee::read_snapshot(&project.root_path);
    if !snapshot.present {
        return not_found("this project has no .bee/ store");
    }

    let by_feature = |cells: &[mdview_core::bee::BeeCell]| -> Vec<mdview_core::bee::BeeCell> {
        cells.iter().filter(|c| c.feature == feature).cloned().collect()
    };
    let buckets = mdview_core::bee::BeeBuckets {
        doing: by_feature(&snapshot.buckets.doing),
        waiting: by_feature(&snapshot.buckets.waiting),
        stuck: by_feature(&snapshot.buckets.stuck),
        done: by_feature(&snapshot.buckets.done),
    };
    let shipped = snapshot.shipped.iter().find(|f| f.feature == feature);

    let known_feature = shipped.is_some()
        || !buckets.doing.is_empty()
        || !buckets.waiting.is_empty()
        || !buckets.stuck.is_empty()
        || !buckets.done.is_empty();
    if !known_feature {
        return not_found("feature not found");
    }

    Html(views::bee_feature_page(&project, &feature, &buckets, shipped)).into_response()
}

/// Render `s` relative to `root` when it names a path under `root`; reduce
/// to its bare filename when it is absolute but falls outside `root`. A
/// local twin of `mdview_core::bee`'s private `relativize` — that module is
/// out of scope for this cell, and the detail routes read the raw cell JSON
/// directly (to reach fields the trimmed `BeeCell` doesn't carry), so they
/// need their own copy of the same no-absolute-path contract.
fn relativize_detail_path(s: &str, root: &std::path::Path) -> String {
    let p = std::path::Path::new(s);
    if !p.is_absolute() {
        return s.to_string();
    }
    match p.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => p
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(absolute path redacted)".to_string()),
    }
}

/// Scan the live `.bee/cells/*.json` tree (never `archive/`, per D9) for the
/// JSON object whose own `"id"` field matches `cell_id` — filenames are not
/// guaranteed to match a cell's id (see `bee_route_tests::cell_json`, which
/// deliberately writes `a.json`/`b.json` carrying ids like `c-open`), so a
/// direct `<cells_dir>/<cell_id>.json` lookup would miss real cells.
fn find_cell_full(root: &std::path::Path, cell_id: &str) -> Option<views::BeeCellFull> {
    let cells_dir = root.join(".bee").join("cells");
    if !cells_dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(&cells_dir).ok()?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if v.get("id").and_then(serde_json::Value::as_str) != Some(cell_id) {
            continue;
        }
        return Some(cell_full_from_json(&v, root));
    }
    None
}

/// Parse one raw `.bee/cells/<id>.json` object into a [`views::BeeCellFull`],
/// relativizing every path-shaped value it carries (files, read_first,
/// trace.worker, trace.results) the same way `mdview_core::bee::parse_cell`
/// does for the trimmed board `BeeCell`.
fn cell_full_from_json(v: &serde_json::Value, root: &std::path::Path) -> views::BeeCellFull {
    use serde_json::Value;

    let str_field = |key: &str| -> String {
        v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    };
    let opt_str_field = |key: &str| -> Option<String> {
        v.get(key).and_then(Value::as_str).map(String::from)
    };
    let str_array = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default()
    };
    let path_array = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| relativize_detail_path(s, root))
                    .collect()
            })
            .unwrap_or_default()
    };

    let must_have_truths = v
        .get("must_haves")
        .and_then(|m| m.get("truths"))
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default();

    let trace = v.get("trace");
    let worker = trace
        .and_then(|t| t.get("worker"))
        .and_then(Value::as_str)
        .map(|w| relativize_detail_path(w, root));
    let claimed_at = trace
        .and_then(|t| t.get("claimed_at"))
        .and_then(Value::as_str)
        .map(String::from);
    let capped_at = trace
        .and_then(|t| t.get("capped_at"))
        .and_then(Value::as_str)
        .map(String::from);
    let outcome = trace
        .and_then(|t| t.get("outcome"))
        .and_then(Value::as_str)
        .map(String::from);
    // A deviation entry is either a plain string or an object carrying a
    // "description" (see beehive's `wl-2.json`, `p2-2.json`) — both shapes
    // are folded to a display string here, and anything else falls back to
    // its raw JSON rather than silently dropping the entry.
    let deviations = trace
        .and_then(|t| t.get("deviations"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|d| {
                    d.as_str()
                        .map(String::from)
                        .or_else(|| d.get("description").and_then(Value::as_str).map(String::from))
                        .unwrap_or_else(|| d.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    let tests = trace.and_then(|t| t.get("tests")).and_then(Value::as_str).map(String::from);
    let results = trace
        .and_then(|t| t.get("results"))
        .and_then(Value::as_str)
        .map(|r| relativize_detail_path(r, root));

    views::BeeCellFull {
        id: str_field("id"),
        feature: str_field("feature"),
        title: str_field("title"),
        action: str_field("action"),
        verify: str_field("verify"),
        lane: str_field("lane"),
        status: str_field("status"),
        tier: opt_str_field("tier"),
        files: path_array("files"),
        read_first: path_array("read_first"),
        decisions: str_array("decisions"),
        must_have_truths,
        worker,
        claimed_at,
        capped_at,
        outcome,
        deviations,
        tests,
        results,
    }
}

/// Which file a project opens to — a fixed, predictable rule instead of
/// "whatever the index lists first". Precedence: a `README.md` wins over
/// everything, then an `index.md`, then any other file; within the same rank the
/// shallowest path wins, then case-insensitive alphabetical order. So a
/// project's README is the landing page when it has one, and the choice never
/// looks random.
fn pick_entry_file(
    files: &[mdview_core::domain::IndexedFile],
) -> Option<&mdview_core::domain::IndexedFile> {
    fn rank(rel: &str) -> u8 {
        match rel
            .rsplit('/')
            .next()
            .unwrap_or(rel)
            .to_ascii_lowercase()
            .as_str()
        {
            "readme.md" => 0,
            "index.md" => 1,
            _ => 2,
        }
    }
    fn depth(rel: &str) -> usize {
        rel.bytes().filter(|&b| b == b'/').count()
    }
    files.iter().min_by(|a, b| {
        rank(&a.rel_path)
            .cmp(&rank(&b.rel_path))
            .then_with(|| depth(&a.rel_path).cmp(&depth(&b.rel_path)))
            .then_with(|| {
                a.rel_path
                    .to_ascii_lowercase()
                    .cmp(&b.rel_path.to_ascii_lowercase())
            })
    })
}

async fn project_path(
    State(st): State<AppState>,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    // Markdown file in the index → render it.
    if let Ok(Some(project)) = st.engine.get_project(&id) {
        if st
            .engine
            .store
            .get_file(&id, &path)
            .ok()
            .flatten()
            .is_some()
        {
            return match st.engine.render_file(&id, &path) {
                Ok(page) => {
                    let file = st.engine.store.get_file(&id, &path).unwrap().unwrap();
                    let files = st.engine.list_files(&id).unwrap_or_default();
                    let backlinks = st.engine.backlinks(&id, &path).unwrap_or_default();
                    Html(views::file_page(&project, &file, &page, &files, &backlinks))
                        .into_response()
                }
                Err(e) => internal_error(&e.to_string()),
            };
        }
        // Otherwise serve as a static asset (image, etc.) with traversal guard.
        if let Ok(abs) = st.engine.asset_path(&id, &path) {
            if let Ok(bytes) = std::fs::read(&abs) {
                return asset_response(&abs, bytes);
            }
        }
    }
    not_found("file not found")
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
}

async fn search_page(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    let results = if query.q.trim().is_empty() {
        Vec::new()
    } else {
        st.engine
            .search(&query.q, Some(&id), 30)
            .unwrap_or_default()
    };
    Html(views::search_page(&project, &query.q, &results)).into_response()
}

#[derive(serde::Deserialize)]
struct JumpQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_jump_limit")]
    limit: usize,
}

fn default_jump_limit() -> usize {
    20
}

/// Fuzzy file-jump endpoint: ranks the project's files by a fuzzy match of `q`
/// against their relative paths (complements the `_search` content search) and
/// returns the hits as JSON for the client jump palette.
async fn jump_search(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<JumpQuery>,
) -> Response {
    if matches!(st.engine.get_project(&id), Ok(None) | Err(_)) {
        return not_found("project not found");
    }
    let hits = st
        .engine
        .fuzzy_files(&id, &query.q, query.limit)
        .unwrap_or_default();
    Json(hits).into_response()
}

async fn ws_handler(ws: WebSocketUpgrade, State(st): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, st.reload_tx.subscribe()))
}

async fn handle_ws(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    loop {
        tokio::select! {
            r = rx.recv() => match r {
                Ok(msg) => {
                    if socket.send(Message::Text(msg)).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            r = socket.recv() => match r {
                Some(Ok(_)) => {}
                _ => break,
            },
        }
    }
}

async fn bind_with_retry(host: &str, port: u16) -> Result<(tokio::net::TcpListener, SocketAddr)> {
    for p in port..port.saturating_add(10) {
        let addr = format!("{host}:{p}");
        if let Ok(l) = tokio::net::TcpListener::bind(&addr).await {
            let local = l.local_addr()?;
            return Ok((l, local));
        }
    }
    anyhow::bail!("no free port in {port}..{}", port + 10);
}

fn build_highlight_css(engine: &Engine) -> String {
    // Atelier renders code blocks (`.fg-prose pre`) on a fixed dark "signature"
    // panel in both page schemes (D5), so syntect must emit a dark palette that
    // stays readable on that panel whether the page is in light or dark scheme.
    // Scope the same dark theme under both data-scheme values rather than
    // pairing a light theme with the light scheme.
    let dark = theme_css("base16-ocean.dark").unwrap_or_default();
    let _ = &engine.config.renderer.syntax_highlight_theme; // reserved for user override
    format!(
        "{}\n{}",
        scope_css(&dark, ":root[data-scheme=\"light\"]"),
        scope_css(&dark, ":root[data-scheme=\"dark\"]")
    )
}

/// Prefix every selector in `css` with `prefix` so two theme sheets coexist.
fn scope_css(css: &str, prefix: &str) -> String {
    let css = strip_comments(css);
    let mut out = String::new();
    for block in css.split_inclusive('}') {
        if let Some(idx) = block.find('{') {
            let (sel, rest) = block.split_at(idx);
            let scoped = sel
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| format!("{prefix} {s}"))
                .collect::<Vec<_>>()
                .join(", ");
            if !scoped.is_empty() {
                out.push_str(&scoped);
                out.push(' ');
                out.push_str(rest);
            }
        }
    }
    out
}

fn strip_comments(css: &str) -> String {
    let mut out = String::new();
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find("*/") {
            rest = &rest[start + end + 2..];
        } else {
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn content_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("bmp") => "image/bmp",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Build the HTTP response for a static project asset.
///
/// Assets are project-supplied bytes served on a no-auth origin and do NOT pass
/// through the markdown sanitizer. `X-Content-Type-Options: nosniff` plus a
/// fully-restrictive `Content-Security-Policy: sandbox` stop a project-supplied
/// `.svg` (served as `image/svg+xml`) from executing script when navigated to
/// directly, while still letting it render inside an `<img>`.
fn asset_response(path: &std::path::Path, bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type(path)),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_SECURITY_POLICY, "sandbox"),
        ],
        bytes,
    )
        .into_response()
}

/// Normalize a submitted `hostname`: trim it and treat blank/whitespace-only as
/// unset. The settings form always sends the field (empty when cleared), so this
/// maps `""`/`"  "` → `None` and keeps the display override off `http://:PORT`.
fn normalize_hostname(raw: Option<String>) -> Option<String> {
    raw.map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
}

/// True when `host` is a loopback bind (safe default). A wildcard (`0.0.0.0`/`::`)
/// or a concrete LAN IP is not loopback and exposes the no-auth server to the
/// network — the trigger for the startup warning.
fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Html(views::error_page(404, msg))).into_response()
}
fn internal_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(views::error_page(500, msg)),
    )
        .into_response()
}

#[cfg(test)]
mod highlight_css_tests {
    use super::*;

    #[test]
    fn dark_theme_is_scoped_to_both_schemes_without_page_wide_background() {
        let dark = theme_css("base16-ocean.dark").unwrap_or_default();
        let scoped = format!(
            "{}\n{}",
            scope_css(&dark, ":root[data-scheme=\"light\"]"),
            scope_css(&dark, ":root[data-scheme=\"dark\"]")
        );
        assert!(scoped.contains(":root[data-scheme=\"light\"]"));
        assert!(scoped.contains(":root[data-scheme=\"dark\"]"));
        // Every scoped selector must target something under the prefix, never
        // the bare :root itself, or the theme's background would leak page-wide.
        assert!(!scoped.contains(":root[data-scheme=\"light\"] {"));
        assert!(!scoped.contains(":root[data-scheme=\"dark\"] {"));
    }
}

#[cfg(test)]
mod asset_response_tests {
    use super::*;

    #[test]
    fn svg_asset_is_sandboxed_and_nosniff() {
        // A project-supplied .svg must be served with headers that neutralize
        // script execution on direct navigation (the XSS vector).
        let resp = asset_response(std::path::Path::new("diagram.svg"), b"<svg/>".to_vec());
        let h = resp.headers();
        assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "image/svg+xml");
        assert_eq!(h.get(header::CONTENT_SECURITY_POLICY).unwrap(), "sandbox");
        assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
    }

    #[test]
    fn png_asset_also_carries_security_headers() {
        let resp = asset_response(std::path::Path::new("logo.png"), b"x".to_vec());
        let h = resp.headers();
        assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "image/png");
        assert_eq!(h.get(header::CONTENT_SECURITY_POLICY).unwrap(), "sandbox");
        assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
    }

    #[test]
    fn project_summary_json_omits_filesystem_path() {
        let v = project_summary_json("abc", "My Proj", 3);
        assert!(
            v.get("root_path").is_none(),
            "unauthenticated API must not leak the project filesystem path"
        );
        assert_eq!(v["id"], "abc");
        assert_eq!(v["name"], "My Proj");
        assert_eq!(v["file_count"], 3);
        assert_eq!(v["url"], "/p/abc/");
    }

    #[test]
    fn hostname_form_value_normalizes_blank_to_none() {
        assert_eq!(normalize_hostname(None), None);
        assert_eq!(normalize_hostname(Some(String::new())), None);
        assert_eq!(normalize_hostname(Some("   ".into())), None);
        assert_eq!(
            normalize_hostname(Some("  host.local ".into())),
            Some("host.local".to_string())
        );
    }

    fn f(rel: &str) -> mdview_core::domain::IndexedFile {
        mdview_core::domain::IndexedFile {
            project_id: "p".into(),
            abs_path: std::path::PathBuf::from(rel),
            rel_path: rel.into(),
            title: rel.into(),
            size_bytes: 0,
            modified_at: String::new(),
        }
    }

    #[test]
    fn entry_file_prefers_readme_then_index_then_shallow_alpha() {
        // README wins even when a non-README sorts earlier alphabetically.
        let files = vec![f("architecture.md"), f("README.md"), f("guide.md")];
        assert_eq!(pick_entry_file(&files).unwrap().rel_path, "README.md");

        // README anywhere beats a root-level non-README (README is the rule).
        let files = vec![f("guide.md"), f("docs/README.md")];
        assert_eq!(pick_entry_file(&files).unwrap().rel_path, "docs/README.md");

        // A shallower README beats a deeper one.
        let files = vec![f("docs/README.md"), f("README.md")];
        assert_eq!(pick_entry_file(&files).unwrap().rel_path, "README.md");

        // No README → index.md wins.
        let files = vec![f("zoo.md"), f("index.md"), f("apple.md")];
        assert_eq!(pick_entry_file(&files).unwrap().rel_path, "index.md");

        // Neither → shallowest, then alphabetical.
        let files = vec![f("docs/a.md"), f("beta.md"), f("alpha.md")];
        assert_eq!(pick_entry_file(&files).unwrap().rel_path, "alpha.md");

        // Case-insensitive basename match.
        let files = vec![f("intro.md"), f("ReadMe.md")];
        assert_eq!(pick_entry_file(&files).unwrap().rel_path, "ReadMe.md");
    }

    #[test]
    fn loopback_detection_flags_wildcard_and_lan_as_exposed() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("192.168.1.10"));
        assert!(!is_loopback_host("::"));
    }
}

/// Route-level tests for `GET /p/:id/_bee` and the D3 project-home gate
/// (bee-cockpit-2). Every test here drives `router()` through
/// `tower::ServiceExt::oneshot` — the harness this crate had none of before
/// this cell — because the "not found, not an empty page" half of D3 is a
/// routing decision no pure view-function assertion can prove.
#[cfg(test)]
mod bee_route_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use mdview_core::domain::Project;
    use mdview_core::{Config, SqliteStore};
    use std::path::{Path, PathBuf};
    use tower::ServiceExt;

    fn fresh_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mdview-server-bee-{name}-{}-{}",
            std::process::id(),
            name.len(), // cheap per-name salt, keeps directories distinct across test fns
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn cell_json(id: &str, status: &str, files: &[String], worker: &str) -> String {
        let files_json = files
            .iter()
            .map(|f| serde_json::to_string(f).unwrap())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{
                "id": "{id}",
                "feature": "demo",
                "lane": "standard",
                "title": "Cell {id}",
                "action": "do the thing",
                "verify": "cargo test",
                "files": [{files_json}],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {{}},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "{status}",
                "tier": "generation",
                "trace": {{"worker": {worker_json}}}
            }}"#,
            worker_json = serde_json::to_string(worker).unwrap(),
        )
    }

    fn build_state() -> AppState {
        let engine = Arc::new(Engine::new(
            SqliteStore::open_in_memory().unwrap(),
            Config::default(),
        ));
        let (reload_tx, _) = broadcast::channel(4);
        AppState {
            engine,
            reload_tx,
            highlight_css: Arc::new(String::new()),
            config_data_dir: None,
            terminal_auth: TerminalAuth::new(None),
        }
    }

    /// `build_state()` plus both `config_data_dir` and `terminal_auth`
    /// pointed at the same scratch `dir` — the token file lives beside
    /// `config.toml` (see `terminal_auth::token_path_override`), so a test
    /// that only overrides one of the two silently reads/writes the token at
    /// the real `~/.mdview` instead of its scratch dir.
    fn build_state_with_dir(dir: &Path) -> AppState {
        let mut st = build_state();
        st.config_data_dir = Some(dir.to_path_buf());
        st.terminal_auth = TerminalAuth::new(Some(dir.to_path_buf()));
        st
    }

    fn register(st: &AppState, root: &Path, name: &str) -> Project {
        st.engine.register(root, Some(name)).unwrap()
    }

    async fn get(app: Router, uri: &str) -> Response {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn body_string(resp: Response) -> String {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// (relative path, content bytes) for every file under `dir` — the D4
    /// read-only probe's before/after snapshot.
    fn snapshot_tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in std::fs::read_dir(cur).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                    let content = std::fs::read(&path).unwrap();
                    out.push((rel, content));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[tokio::test]
    async fn happy_path_returns_200_with_bucket_counts() {
        let root = fresh_root("happy");
        write(&root, "README.md", "# hi");
        write(
            &root,
            ".bee/state.json",
            r#"{"phase":"swarming","feature":"demo","mode":"standard"}"#,
        );
        write(&root, ".bee/cells/a.json", &cell_json("c-open", "open", &[], "w1"));
        write(
            &root,
            ".bee/cells/b.json",
            &cell_json("c-claimed", "claimed", &[], "w1"),
        );
        write(
            &root,
            ".bee/cells/c.json",
            &cell_json("c-blocked", "blocked", &[], "w1"),
        );
        write(
            &root,
            ".bee/cells/d.json",
            &cell_json("c-capped-1", "capped", &[], "w1"),
        );
        write(
            &root,
            ".bee/cells/e.json",
            &cell_json("c-capped-2", "capped", &[], "w1"),
        );

        let st = build_state();
        let project = register(&st, &root, "happy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("data-bucket=\"doing\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"waiting\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"stuck\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"done\" data-count=\"2\""), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn empty_cells_dir_yields_four_zero_buckets() {
        let root = fresh_root("empty-cells");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let st = build_state();
        let project = register(&st, &root, "empty-cells");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        for key in ["doing", "waiting", "stuck", "done"] {
            assert!(
                body.contains(&format!("data-bucket=\"{key}\" data-count=\"0\"")),
                "expected a zero {key} bucket: {body}"
            );
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn no_bee_dir_is_not_found_route_and_no_home_page_entry_point() {
        // No .bee/ and no markdown files either, so project_home renders a
        // real body (the "no markdown files" not-found page) instead of a
        // redirect — the exact branch that WOULD carry the bee entry point
        // link if `.bee/` were present (see the positive control below).
        let root = fresh_root("no-bee");

        let st = build_state();
        let project = register(&st, &root, "no-bee");
        let app = router(st);

        let bee_resp = get(app.clone(), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(bee_resp.status(), StatusCode::NOT_FOUND);

        let home_resp = get(app, &format!("/p/{}/", project.id)).await;
        let home_body = body_string(home_resp).await;
        assert!(
            !home_body.contains("_bee"),
            "no-.bee/ project home page must carry no bee entry point: {home_body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn bee_dir_present_puts_entry_point_on_project_home_page() {
        // Positive control for the test above: same shape (no markdown
        // files), but with `.bee/` present — proves the branch that omits
        // the entry point when absent is the same branch that emits it when
        // present, not two unrelated code paths.
        let root = fresh_root("home-entry");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);
        write(&root, ".bee/cells/a.json", &cell_json("c1", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "home-entry");
        let resp = get(router(st), &format!("/p/{}/", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee\"", project.id)),
            "project home page must link to the bee board when .bee/ is present: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn no_absolute_path_or_fixture_root_in_response_body() {
        let root = fresh_root("security");
        let root_str = root.to_string_lossy().into_owned();
        let inside_abs = root
            .join("src/inside.rs")
            .to_string_lossy()
            .into_owned();
        let outside_abs = std::env::temp_dir()
            .join("mdview-server-bee-outside-file.rs")
            .to_string_lossy()
            .into_owned();
        let worker_abs = root
            .join("workers/reader-1")
            .to_string_lossy()
            .into_owned();

        write(
            &root,
            ".bee/cells/leaky.json",
            &cell_json(
                "leaky",
                "open",
                &[inside_abs.clone(), outside_abs.clone()],
                &worker_abs,
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "security");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            !body.contains(&root_str),
            "response body leaked the fixture root: {body}"
        );
        assert!(
            !body.contains(&outside_abs),
            "response body leaked an absolute path outside the fixture root: {body}"
        );
        assert!(
            !body.contains(&worker_abs),
            "response body leaked the absolute worker path: {body}"
        );
        assert!(
            !body.contains(&inside_abs),
            "response body leaked the in-root absolute path verbatim (should be relativized): {body}"
        );
        // The board no longer prints a cell's file list at all (that detail
        // moved to the cell detail page) — so the in-root file must not
        // appear even in its clean, relativized form here.
        assert!(
            !body.contains("src/inside.rs"),
            "board card leaked a cell's file path — file lists belong on the cell detail page only: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn reading_never_writes_the_fixtures_bee_tree() {
        let root = fresh_root("read-only");
        write(&root, "README.md", "# hi");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);
        write(&root, ".bee/cells/a.json", &cell_json("a", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "read-only");
        let before = snapshot_tree(&root);

        let _ = get(router(st), &format!("/p/{}/_bee", project.id)).await;

        let after = snapshot_tree(&root);
        assert_eq!(before, after, ".bee/ tree changed after a request");

        std::fs::remove_dir_all(&root).ok();
    }

    /// Like `cell_json`, plus `trace.claimed_at`/`trace.capped_at` — the two
    /// timestamps D11's cycle-time math needs. Used only by the velocity
    /// tests below (bee-cockpit-4); the plain `cell_json` above stays
    /// untouched so every bee-cockpit-2 test keeps its exact fixture shape.
    fn timed_cell_json(
        id: &str,
        feature: &str,
        status: &str,
        files: &[String],
        worker: &str,
        claimed_at: &str,
        capped_at: &str,
    ) -> String {
        let files_json = files
            .iter()
            .map(|f| serde_json::to_string(f).unwrap())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{
                "id": "{id}",
                "feature": "{feature}",
                "lane": "standard",
                "title": "Cell {id}",
                "action": "do the thing",
                "verify": "cargo test",
                "files": [{files_json}],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {{}},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "{status}",
                "tier": "generation",
                "trace": {{"worker": {worker_json}, "claimed_at": "{claimed_at}", "capped_at": "{capped_at}"}}
            }}"#,
            worker_json = serde_json::to_string(worker).unwrap(),
        )
    }

    /// bee-cockpit-4 (happy): a fixture with one fully-capped, timed feature
    /// (24 minutes claim-to-cap → 0.4h, matching the ground-truth beehive
    /// numbers) plus one still-open feature. The body must carry all three
    /// headline numbers in plain language and both feature lists.
    #[tokio::test]
    async fn velocity_headline_numbers_and_feature_lists_render_for_shipped_work() {
        let root = fresh_root("velocity-happy");
        write(
            &root,
            ".bee/cells/shipped.json",
            &timed_cell_json(
                "s1",
                "demo",
                "capped",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );
        // `cell_json`'s fixed "demo" feature would collide with the shipped
        // one above and hide the "still open" case, so this uses
        // `timed_cell_json` with a distinct feature instead (its timestamps
        // are unused while the cell stays "open").
        write(
            &root,
            ".bee/cells/open.json",
            &timed_cell_json(
                "o1",
                "still-cooking",
                "open",
                &[],
                "w1",
                "2026-08-04T09:00:00Z",
                "2026-08-04T09:00:00Z",
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "velocity-happy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Shipped per working day"), "{body}");
        assert!(body.contains("Shipped per week"), "{body}");
        assert!(body.contains("Typical time to finish"), "{body}");
        assert!(body.contains("0.4h"), "median cycle time missing: {body}");
        assert!(body.contains("demo"), "shipped feature name missing: {body}");
        assert!(
            body.contains("still-cooking"),
            "open feature name missing: {body}"
        );
        assert!(!body.contains("NaN"), "{body}");
        assert!(!body.contains("Infinity"), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-4 (edge): nothing has shipped. The section must render an
    /// honest empty state — no zeroed-out numbers, no NaN, no Infinity, no
    /// division artifact anywhere in the body.
    #[tokio::test]
    async fn no_shipped_features_renders_honest_empty_state_not_zeros() {
        let root = fresh_root("velocity-empty");
        write(&root, ".bee/cells/a.json", &cell_json("a", "open", &[], "w1"));
        write(
            &root,
            ".bee/cells/b.json",
            &cell_json("b", "blocked", &[], "w1"),
        );

        let st = build_state();
        let project = register(&st, &root, "velocity-empty");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("No features have shipped yet"),
            "expected an honest empty state: {body}"
        );
        assert!(!body.contains("NaN"), "{body}");
        assert!(!body.contains("Infinity"), "{body}");
        assert!(!body.contains("0.0"), "a zero stat leaked in: {body}");
        assert!(!body.contains("0/0"), "a division artifact leaked in: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-4 / bee-board-ux-2 (security): a finished ("capped") cell
    /// still carries the same path-leak risk as any other cell — this proves
    /// the collapsed Done section (`bee_done_section`, the sole surviving
    /// finished-work list since `bee_shipped_list` was removed) is reached
    /// by the same fixture and still leaks nothing. Named against the
    /// fixture's own root and `std::env::temp_dir()`, never a production
    /// literal like `/home/`
    /// (per `docs/history/learnings/20260805-toothless-security-assertions.md`).
    #[tokio::test]
    async fn finished_feature_cell_paths_do_not_leak_into_done_section() {
        let root = fresh_root("done-section-security");
        let root_str = root.to_string_lossy().into_owned();
        let inside_abs = root.join("src/inside.rs").to_string_lossy().into_owned();
        let outside_abs = std::env::temp_dir()
            .join("mdview-server-bee-done-section-outside.rs")
            .to_string_lossy()
            .into_owned();
        let worker_abs = root.join("workers/reader-1").to_string_lossy().into_owned();

        write(
            &root,
            ".bee/cells/leaky.json",
            &timed_cell_json(
                "leaky",
                "leaky-feature",
                "capped",
                &[inside_abs.clone(), outside_abs.clone()],
                &worker_abs,
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "done-section-security");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            !body.contains(&root_str),
            "response body leaked the fixture root: {body}"
        );
        assert!(
            !body.contains(&outside_abs),
            "response body leaked an absolute path outside the fixture root: {body}"
        );
        assert!(
            !body.contains(&worker_abs),
            "response body leaked the absolute worker path: {body}"
        );
        assert!(
            !body.contains(&inside_abs),
            "response body leaked the in-root absolute path verbatim: {body}"
        );
        assert!(body.contains("leaky-feature"), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-4 (read-only): the velocity section reads the same
    /// `.bee/cells/*.json` files as the buckets, over a fixture that has a
    /// shipped feature (so D10/D11's grouping and cycle-time math actually
    /// run). D4 must hold end to end: the tree is byte-identical before and
    /// after the request.
    #[tokio::test]
    async fn velocity_read_never_writes_the_fixtures_bee_tree() {
        let root = fresh_root("velocity-read-only");
        write(&root, "README.md", "# hi");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);
        write(
            &root,
            ".bee/cells/shipped.json",
            &timed_cell_json(
                "s1",
                "demo",
                "capped",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );
        write(&root, ".bee/cells/open.json", &cell_json("a", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "velocity-read-only");
        let before = snapshot_tree(&root);

        let _ = get(router(st), &format!("/p/{}/_bee", project.id)).await;

        let after = snapshot_tree(&root);
        assert_eq!(before, after, ".bee/ tree changed after a request");

        std::fs::remove_dir_all(&root).ok();
    }

    // ── bee-cockpit-6: backlog / sessions / lanes panels ──────────────────

    fn session_json(id: &str, last_heartbeat: &str, transcript_path: &str, workspace_id: &str, source: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "started_at": "2026-08-04T08:00:00Z",
                "last_heartbeat": "{last_heartbeat}",
                "transcript_path": "{transcript_path}",
                "workspace_id": "{workspace_id}",
                "source": "{source}"
            }}"#,
            transcript_path = transcript_path.replace('\\', "\\\\"),
        )
    }

    fn lane_json(feature: &str, phase: &str, mode: &str, next_action: &str) -> String {
        format!(
            r#"{{"feature": "{feature}", "phase": "{phase}", "mode": "{mode}", "next_action": "{next_action}"}}"#
        )
    }

    fn workspace_json(id: &str, root: &str, branch: &str, attached: &[&str]) -> String {
        let attached_json = attached
            .iter()
            .map(|s| serde_json::to_string(s).unwrap())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{
                "id": "{id}",
                "type": "worktree",
                "root": "{root}",
                "branch": "{branch}",
                "attached_sessions": [{attached_json}],
                "created_at": "2026-08-04T08:00:00Z"
            }}"#,
            root = root.replace('\\', "\\\\"),
        )
    }

    fn rfc3339_minutes_ago(mins: i64) -> String {
        let now = time::OffsetDateTime::now_utc();
        (now - time::Duration::minutes(mins))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    }

    /// bee-cockpit-6 (happy): a fixture carrying backlog PBIs/findings, one
    /// live and one stale session, and a lane + workspace. The body must
    /// carry the PBI statuses, the severity counts, and both sessions'
    /// liveness in plain language.
    #[tokio::test]
    async fn panels_render_backlog_sessions_and_lanes_with_liveness() {
        let root = fresh_root("panels-happy");
        write(
            &root,
            ".bee/backlog.jsonl",
            "{\"kind\":\"pbi\",\"id\":\"PBI-1\",\"title\":\"Add search\",\"status\":\"in-flight\",\"feature\":\"demo\"}\n\
             {\"kind\":\"pbi\",\"id\":\"PBI-2\",\"title\":\"Add filter\",\"status\":\"done\",\"feature\":\"demo\"}\n\
             {\"ts\":\"2026-08-05T04:00:00Z\",\"type\":\"finding\",\"title\":\"Race in write path\",\"detail\":\"d\",\"severity\":\"P1\",\"layer\":\"server\",\"feature\":\"demo\"}\n\
             {\"ts\":\"2026-08-05T03:00:00Z\",\"type\":\"finding\",\"title\":\"Slow query\",\"detail\":\"d\",\"severity\":\"P2\",\"layer\":\"db\",\"feature\":\"demo\"}\n",
        );
        write(
            &root,
            ".bee/sessions/live.json",
            &session_json("sess-live", &rfc3339_minutes_ago(4), "/home/x/transcript-live.json", "ws-1", "claude"),
        );
        write(
            &root,
            ".bee/sessions/stale.json",
            &session_json("sess-stale", &rfc3339_minutes_ago(120), "/home/x/transcript-stale.json", "ws-2", "codex"),
        );
        write(&root, ".bee/lanes/demo.json", &lane_json("demo", "swarming", "standard", "run tests"));
        write(
            &root,
            ".bee/runtime/workspaces/ws-1.json",
            &workspace_json("ws-1", "demo--wt--feature", "wt/demo", &["sess-live"]),
        );

        let st = build_state();
        let project = register(&st, &root, "panels-happy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        // PBI statuses.
        assert!(body.contains("in-flight: 1"), "{body}");
        assert!(body.contains("done: 1"), "{body}");
        // Severity counts.
        assert!(body.contains("P1: 1"), "{body}");
        assert!(body.contains("P2: 1"), "{body}");
        assert!(body.contains("P3: 0"), "{body}");
        // Session liveness, plain language, no raw timestamp.
        assert!(body.contains("live"), "{body}");
        assert!(body.contains("stale"), "{body}");
        assert!(body.contains("4 minutes ago"), "{body}");
        assert!(body.contains("2 hours ago"), "{body}");
        assert!(!body.contains("T04:"), "raw ISO timestamp leaked into a heartbeat: {body}");
        // Lane + workspace.
        assert!(body.contains("wt/demo"), "{body}");
        assert!(body.contains("swarming"), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 (happy): 25 findings exceed `RECENT_DETAIL_CAP` (20), so
    /// the findings panel must state its true total (25) alongside the
    /// capped count actually shown, not just the visible subset.
    #[tokio::test]
    async fn capped_findings_subset_states_its_true_total() {
        let root = fresh_root("panels-capped");
        let mut jsonl = String::new();
        for i in 0..25 {
            jsonl.push_str(&format!(
                "{{\"ts\":\"2026-08-05T04:{i:02}:00Z\",\"type\":\"finding\",\"title\":\"Finding {i}\",\"detail\":\"d\",\"severity\":\"P3\",\"layer\":\"x\",\"feature\":\"demo\"}}\n"
            ));
        }
        write(&root, ".bee/backlog.jsonl", &jsonl);

        let st = build_state();
        let project = register(&st, &root, "panels-capped");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("Showing 20 of 25 findings."),
            "capped subset must state its true total: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 (edge): none of backlog/sessions/lanes/workspaces exist.
    /// The three panels must each render an honest empty state — no hidden
    /// panel, no bare `0` standing in for missing data.
    #[tokio::test]
    async fn absent_backlog_sessions_and_lanes_render_honest_empty_states() {
        let root = fresh_root("panels-empty");
        write(&root, "README.md", "# hi");
        // A present-but-empty `.bee/` (D3) — no `.bee/` at all would 404
        // instead of rendering the honest-empty-state panels under test.
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let st = build_state();
        let project = register(&st, &root, "panels-empty");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("No backlog items yet."), "{body}");
        assert!(body.contains("No findings yet."), "{body}");
        assert!(body.contains("No sessions recorded."), "{body}");
        assert!(body.contains("No lanes running."), "{body}");
        assert!(body.contains("No worktree workspaces yet."), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 (security): a session's `transcript_path` must never
    /// reach the body, and no absolute path — the fixture root itself, or a
    /// workspace's absolute `root` field — may survive verbatim either.
    /// Named against the fixture's own root and `std::env::temp_dir()`, per
    /// `docs/history/learnings/20260805-toothless-security-assertions.md`,
    /// never a production literal like `/home/`.
    #[tokio::test]
    async fn panels_leak_no_transcript_path_and_no_absolute_path() {
        let root = fresh_root("panels-security");
        let root_str = root.to_string_lossy().into_owned();
        let transcript_abs = root.join(".bee/sessions/leaky-transcript.json").to_string_lossy().into_owned();
        let outside_workspace_root = std::env::temp_dir()
            .join("mdview-server-bee-panels-outside-workspace")
            .to_string_lossy()
            .into_owned();

        write(
            &root,
            ".bee/sessions/leaky.json",
            &session_json("sess-leaky", &rfc3339_minutes_ago(1), &transcript_abs, "ws-out", "claude"),
        );
        write(
            &root,
            ".bee/runtime/workspaces/ws-out.json",
            &workspace_json("ws-out", &outside_workspace_root, "wt/leaky", &[]),
        );

        let st = build_state();
        let project = register(&st, &root, "panels-security");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(!body.contains(&transcript_abs), "transcript_path leaked into the body: {body}");
        assert!(!body.contains(&root_str), "response body leaked the fixture root: {body}");
        assert!(
            !body.contains(&outside_workspace_root),
            "response body leaked an absolute workspace root: {body}"
        );
        assert!(body.contains("sess-leaky"), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 (read-only): the panels read the same on-disk sources
    /// (`backlog.jsonl`, `sessions/`, `lanes/`, `runtime/workspaces/`) as
    /// every other bee-cockpit route (D4) — the fixture tree must stay
    /// byte-identical before and after the request.
    #[tokio::test]
    async fn panels_read_never_writes_the_fixtures_bee_tree() {
        let root = fresh_root("panels-read-only");
        write(&root, "README.md", "# hi");
        write(
            &root,
            ".bee/backlog.jsonl",
            "{\"kind\":\"pbi\",\"id\":\"PBI-1\",\"title\":\"t\",\"status\":\"proposed\",\"feature\":\"demo\"}\n",
        );
        write(
            &root,
            ".bee/sessions/a.json",
            &session_json("sess-a", &rfc3339_minutes_ago(2), "/home/x/t.json", "ws-1", "claude"),
        );
        write(&root, ".bee/lanes/demo.json", &lane_json("demo", "swarming", "standard", "run tests"));
        write(
            &root,
            ".bee/runtime/workspaces/ws-1.json",
            &workspace_json("ws-1", "demo--wt--feature", "wt/demo", &["sess-a"]),
        );

        let st = build_state();
        let project = register(&st, &root, "panels-read-only");
        let before = snapshot_tree(&root);

        let _ = get(router(st), &format!("/p/{}/_bee", project.id)).await;

        let after = snapshot_tree(&root);
        assert_eq!(before, after, ".bee/ tree changed after a request");

        std::fs::remove_dir_all(&root).ok();
    }

    // ── bee-cockpit-7: cell + feature detail pages ─────────────────────────

    /// A cell fixture carrying every field the detail page renders: action,
    /// verify, read_first, decisions, must_haves.truths, and a full trace
    /// (worker, claim/cap timestamps, outcome, deviations, tests, results) —
    /// the fields the board's trimmed `cell_json` fixture never carries.
    fn full_cell_json(id: &str, feature: &str, status: &str, files: &[String], read_first: &[String]) -> String {
        let files_json = files
            .iter()
            .map(|f| serde_json::to_string(f).unwrap())
            .collect::<Vec<_>>()
            .join(",");
        let read_first_json = read_first
            .iter()
            .map(|f| serde_json::to_string(f).unwrap())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{
                "id": "{id}",
                "feature": "{feature}",
                "lane": "standard",
                "title": "Cell {id}",
                "action": "do the full thing",
                "verify": "cargo test --workspace",
                "files": [{files_json}],
                "read_first": [{read_first_json}],
                "deps": [],
                "decisions": ["D1", "D4"],
                "must_haves": {{"truths": ["truth one", "truth two"]}},
                "behavior_change": true,
                "change_class": "behavior",
                "pbi": null,
                "status": "{status}",
                "tier": "standard",
                "trace": {{
                    "worker": "worker-1",
                    "claimed_at": "2026-08-04T08:00:00Z",
                    "capped_at": "2026-08-04T08:24:00Z",
                    "outcome": "shipped the detail pages",
                    "deviations": ["noted a small deviation"],
                    "tests": "green",
                    "results": ".bee/logs/test-results.json"
                }}
            }}"#
        )
    }

    /// (happy) The cell page for a fixture cell returns 200 and carries its
    /// title, action, verify, lane, status, must_haves, decisions cited, and
    /// the full trace (worker, outcome, deviations, test result).
    #[tokio::test]
    async fn cell_detail_page_carries_title_status_lane_and_full_trace() {
        let root = fresh_root("cell-detail-happy");
        write(
            &root,
            ".bee/cells/a.json",
            &full_cell_json("bee-cockpit-7", "bee-cockpit", "capped", &[], &[]),
        );

        let st = build_state();
        let project = register(&st, &root, "cell-detail-happy");
        let resp = get(router(st), &format!("/p/{}/_bee/cell/bee-cockpit-7", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Cell bee-cockpit-7"), "title missing: {body}");
        assert!(body.contains("capped"), "status missing: {body}");
        assert!(body.contains("lane: standard"), "lane missing: {body}");
        assert!(body.contains("do the full thing"), "action missing: {body}");
        assert!(body.contains("cargo test --workspace"), "verify missing: {body}");
        assert!(body.contains("truth one"), "must_haves truth missing: {body}");
        assert!(body.contains("D1"), "decision citation missing: {body}");
        assert!(body.contains("worker-1"), "trace worker missing: {body}");
        assert!(body.contains("shipped the detail pages"), "trace outcome missing: {body}");
        assert!(body.contains("noted a small deviation"), "trace deviation missing: {body}");
        assert!(body.contains("green"), "trace test result missing: {body}");
        // A timestamp reads as relative language, not the raw ISO string
        // it was derived from — see the D4/panels precedent in the test above.
        assert!(
            !body.contains("2026-08-04T08:00:00Z"),
            "raw ISO trace timestamp leaked into the page: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) The feature page returns 200, carries the feature's cells
    /// grouped by the four D7 buckets, and its shipped state (D10) plus
    /// cycle time (D11).
    #[tokio::test]
    async fn feature_detail_page_shows_shipped_state_and_bucket_grouping() {
        let root = fresh_root("feature-detail-happy");
        write(
            &root,
            ".bee/cells/shipped-1.json",
            &timed_cell_json(
                "f1",
                "detail-feature",
                "capped",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "feature-detail-happy");
        let resp = get(router(st), &format!("/p/{}/_bee/feature/detail-feature", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Shipped"), "shipped banner missing: {body}");
        assert!(body.contains("0.4h to finish"), "cycle time missing: {body}");
        assert!(body.contains("data-bucket=\"done\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"doing\" data-count=\"0\""), "{body}");
        assert!(body.contains("data-bucket=\"waiting\" data-count=\"0\""), "{body}");
        assert!(body.contains("data-bucket=\"stuck\" data-count=\"0\""), "{body}");
        assert!(body.contains("Cell f1"), "cell title missing: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// A feature with a live cell in each of Doing/Waiting/Stuck and none
    /// capped groups correctly into all four buckets and is honestly
    /// reported as not shipped (D10 requires every non-dropped cell capped).
    #[tokio::test]
    async fn feature_detail_page_groups_across_all_four_buckets_when_not_shipped() {
        let root = fresh_root("feature-detail-buckets");
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json("g1", "grouped-feature", "open", &[], "w1", "x", "y"),
        );
        write(
            &root,
            ".bee/cells/b.json",
            &timed_cell_json("g2", "grouped-feature", "claimed", &[], "w1", "x", "y"),
        );
        write(
            &root,
            ".bee/cells/c.json",
            &timed_cell_json("g3", "grouped-feature", "blocked", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "feature-detail-buckets");
        let resp = get(router(st), &format!("/p/{}/_bee/feature/grouped-feature", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Not shipped yet"), "{body}");
        assert!(body.contains("data-bucket=\"doing\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"waiting\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"stuck\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"done\" data-count=\"0\""), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// `reachable_links`: rendering the detail pages without linking to them
    /// does not satisfy this cell — the board body must actually carry both
    /// kinds of link.
    #[tokio::test]
    async fn board_body_links_to_cell_and_feature_detail_routes() {
        let root = fresh_root("board-links");
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json(
                "link-cell",
                "link-feature",
                "open",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "board-links");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/cell/link-cell\"", project.id)),
            "board must link cells to their detail page: {body}"
        );
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/feature/link-feature\"", project.id)),
            "board must link features to their detail page: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) The board's Done section groups done cells by feature — one
    /// compact line per feature, not one card per cell — and states the
    /// true total number of done cells, matching `data-count`.
    #[tokio::test]
    async fn board_done_section_groups_by_feature_and_states_true_total() {
        let root = fresh_root("done-grouped");
        write(&root, ".bee/cells/a.json", &cell_json("d1", "capped", &[], "w1"));
        write(&root, ".bee/cells/b.json", &cell_json("d2", "capped", &[], "w1"));
        write(&root, ".bee/cells/c.json", &cell_json("d3", "capped", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "done-grouped");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-bucket=\"done\" data-count=\"3\""), "{body}");
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/feature/demo\"", project.id)),
            "the done feature line must link to the feature detail page: {body}"
        );
        assert!(
            body.contains("3 cells"),
            "board must state the true done-cell total for the feature: {body}"
        );
        // one compact line for the feature, not one card per done cell.
        assert!(!body.contains("Cell d1"), "{body}");
        assert!(!body.contains("Cell d2"), "{body}");
        assert!(!body.contains("Cell d3"), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) bee-board-ux-2: the old `BEE_DONE_FEATURE_CAP` (20) is gone —
    /// the collapsed-by-default Done section no longer truncates, so a
    /// fixture with 25 finished features shows every one of them, none
    /// dropped, and carries no "shown X of Y" note.
    #[tokio::test]
    async fn board_done_section_shows_every_finished_feature_uncapped() {
        let root = fresh_root("done-uncapped");
        for i in 0..25 {
            write(
                &root,
                &format!(".bee/cells/f{i}.json"),
                &timed_cell_json(&format!("d{i}"), &format!("feature-{i:02}"), "capped", &[], "w1", "x", "y"),
            );
        }

        let st = build_state();
        let project = register(&st, &root, "done-uncapped");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-bucket=\"done\" data-count=\"25\""), "{body}");
        assert!(
            !body.contains("Showing"),
            "the done list must no longer be truncated with a shown-vs-total note: {body}"
        );
        for i in 0..25 {
            assert!(
                body.contains(&format!(
                    "href=\"/p/{}/_bee/feature/feature-{:02}\"",
                    project.id, i
                )),
                "feature-{i:02} missing from the uncapped done list: {body}"
            );
        }

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) bee-board-ux-2: the Done section's `<summary>` states both
    /// totals in plain language — the number of finished features and the
    /// number of finished cells — so the collapsed section never
    /// understates the store.
    #[tokio::test]
    async fn board_done_summary_states_feature_and_cell_totals() {
        let root = fresh_root("done-summary-totals");
        write(&root, ".bee/cells/a.json", &cell_json("d1", "capped", &[], "w1"));
        write(&root, ".bee/cells/b.json", &cell_json("d2", "capped", &[], "w1"));
        write(
            &root,
            ".bee/cells/c.json",
            &timed_cell_json("d3", "second-feature", "capped", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "done-summary-totals");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-bucket=\"done\" data-count=\"3\""), "{body}");
        assert!(
            body.contains("2 features finished"),
            "summary must state the finished-feature count: {body}"
        );
        assert!(
            body.contains("3 cells total"),
            "summary must state the finished-cell count: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) bee-board-ux-2: the Done section is a native `<details>`
    /// element carrying no `open` attribute, so it renders collapsed —
    /// no JavaScript involved.
    #[tokio::test]
    async fn board_done_details_element_has_no_open_attribute() {
        let root = fresh_root("done-collapsed");
        write(&root, ".bee/cells/a.json", &cell_json("d1", "capped", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "done-collapsed");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("<details class=\"bee-done-details\">"),
            "expected the Done section as a details element with no open attribute: {body}"
        );
        assert!(
            !body.contains("<details class=\"bee-done-details\" open"),
            "the Done details element must not carry an open attribute (must load collapsed): {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) bee-board-ux-2: the board renders finished work in exactly
    /// one place — the collapsed Done section — never twice. Before this
    /// cell, `bee_shipped_list` rendered the same finished feature a second
    /// time, uncapped, as its own `fg-card` column inside the velocity
    /// section (23 bordered cards on the real beehive store — the thing a
    /// user complained about from a screenshot).
    #[tokio::test]
    async fn board_renders_finished_work_in_exactly_one_place() {
        let root = fresh_root("finished-once");
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json(
                "f1",
                "solo-shipped-feature",
                "capped",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "finished-once");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        // The finished feature's name appears exactly twice — once in its
        // line's `href`, once as the line's visible text. Before this cell,
        // the removed `bee_shipped_list` rendered the same feature a second
        // time (its own href + its own visible text), which would double
        // this count to four.
        assert_eq!(
            body.matches("solo-shipped-feature").count(),
            2,
            "finished feature rendered more than once (the old duplicate shipped list survived): {body}"
        );
        // The velocity section's old per-feature card list heading is gone.
        assert!(
            !body.contains("bee-velocity__subhead\">Shipped"),
            "velocity section must no longer emit its own shipped feature list: {body}"
        );
        // The one surviving list states its totals as the collapsed
        // summary, in plain language, using the word "Shipped".
        assert!(
            body.contains("bee-done-summary\">Shipped"),
            "expected the collapsed Done summary to read \"Shipped: ...\": {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) bee-board-ux-2: a finished feature's line in the Done section
    /// still links to its feature detail page, and a live cell in one of
    /// the other three buckets still links to its cell detail page — both
    /// drill-downs the board exists to reach.
    #[tokio::test]
    async fn board_links_finished_feature_and_live_cell_to_their_detail_pages() {
        let root = fresh_root("done-links");
        write(
            &root,
            ".bee/cells/done.json",
            &timed_cell_json(
                "finished-cell",
                "finished-feature",
                "capped",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );
        write(
            &root,
            ".bee/cells/live.json",
            &timed_cell_json("live-cell", "live-feature", "open", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "done-links");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains(&format!(
                "href=\"/p/{}/_bee/feature/finished-feature\"",
                project.id
            )),
            "the finished feature line must link to the feature detail page: {body}"
        );
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/cell/live-cell\"", project.id)),
            "a live cell must still link to the cell detail page: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A fixture with nothing done renders an honest empty Done
    /// section — not a zero presented as a real measurement.
    #[tokio::test]
    async fn board_done_section_renders_honest_empty_state_when_nothing_done() {
        let root = fresh_root("done-empty");
        write(&root, ".bee/cells/a.json", &cell_json("open-only", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "done-empty");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-bucket=\"done\" data-count=\"0\""), "{body}");
        assert!(body.contains("Nothing done yet."), "{body}");
        // honest empty state: no "N done cell(s) total" note manufactured
        // from a zero.
        assert!(!body.contains("done cell"), "{body}");
        // an honest empty state, not a collapsed empty list — no <details>
        // wrapper when there is nothing to show.
        assert!(
            !body.contains("bee-done-details"),
            "empty Done section must not render as a collapsed empty list: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (regression) A card on the board no longer prints a cell's file
    /// list — that detail moved to the cell detail page, which still shows
    /// it.
    #[tokio::test]
    async fn board_card_drops_file_list_but_cell_detail_page_keeps_it() {
        let root = fresh_root("board-no-files");
        write(
            &root,
            ".bee/cells/a.json",
            &cell_json("has-files", "open", &["src/keep.rs".to_string()], "w1"),
        );

        let st = build_state();
        let project = register(&st, &root, "board-no-files");
        let app = router(st);

        let board_resp = get(app.clone(), &format!("/p/{}/_bee", project.id)).await;
        let board_body = body_string(board_resp).await;
        assert!(
            !board_body.contains("src/keep.rs"),
            "board card must not print a cell's file list: {board_body}"
        );

        let cell_resp = get(app, &format!("/p/{}/_bee/cell/has-files", project.id)).await;
        let cell_body = body_string(cell_resp).await;
        assert!(
            cell_body.contains("src/keep.rs"),
            "cell detail page must still show the file list: {cell_body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) An unknown cell id and an unknown feature name each return a
    /// clean not-found.
    #[tokio::test]
    async fn unknown_cell_id_and_unknown_feature_name_are_not_found() {
        let root = fresh_root("detail-unknown");
        write(&root, ".bee/cells/a.json", &cell_json("real-cell", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "detail-unknown");
        let app = router(st);

        let cell_resp = get(app.clone(), &format!("/p/{}/_bee/cell/does-not-exist", project.id)).await;
        assert_eq!(cell_resp.status(), StatusCode::NOT_FOUND);

        let feature_resp = get(app, &format!("/p/{}/_bee/feature/does-not-exist", project.id)).await;
        assert_eq!(feature_resp.status(), StatusCode::NOT_FOUND);

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A project with no `.bee/` returns not-found from both detail
    /// routes, matching the board (D3).
    #[tokio::test]
    async fn no_bee_dir_returns_not_found_from_both_detail_routes() {
        let root = fresh_root("detail-no-bee");

        let st = build_state();
        let project = register(&st, &root, "detail-no-bee");
        let app = router(st);

        let cell_resp = get(app.clone(), &format!("/p/{}/_bee/cell/anything", project.id)).await;
        assert_eq!(cell_resp.status(), StatusCode::NOT_FOUND);

        let feature_resp = get(app, &format!("/p/{}/_bee/feature/anything", project.id)).await;
        assert_eq!(feature_resp.status(), StatusCode::NOT_FOUND);

        std::fs::remove_dir_all(&root).ok();
    }

    /// (security) Neither detail body contains an absolute path or the
    /// fixture root. Named against the fixture's own root and
    /// `std::env::temp_dir()`, per
    /// `docs/history/learnings/20260805-toothless-security-assertions.md`,
    /// never a production literal like `/home/`.
    #[tokio::test]
    async fn detail_pages_leak_no_absolute_path_or_fixture_root() {
        let root = fresh_root("detail-security");
        let root_str = root.to_string_lossy().into_owned();
        let inside_abs = root.join("src/inside.rs").to_string_lossy().into_owned();
        let outside_abs = std::env::temp_dir()
            .join("mdview-server-bee-detail-outside.rs")
            .to_string_lossy()
            .into_owned();
        let worker_abs = root.join("workers/reader-1").to_string_lossy().into_owned();
        let results_abs = root.join(".bee/logs/test-results.json").to_string_lossy().into_owned();

        let leaky_cell = format!(
            r#"{{
                "id": "leaky-cell",
                "feature": "leaky-feature",
                "lane": "standard",
                "title": "Leaky cell",
                "action": "do the thing",
                "verify": "cargo test",
                "files": [{inside}, {outside}],
                "read_first": [{inside}],
                "deps": [],
                "decisions": [],
                "must_haves": {{}},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "capped",
                "tier": "generation",
                "trace": {{
                    "worker": {worker},
                    "claimed_at": "2026-08-04T08:00:00Z",
                    "capped_at": "2026-08-04T08:24:00Z",
                    "outcome": "ok",
                    "deviations": [],
                    "tests": "green",
                    "results": {results}
                }}
            }}"#,
            inside = serde_json::to_string(&inside_abs).unwrap(),
            outside = serde_json::to_string(&outside_abs).unwrap(),
            worker = serde_json::to_string(&worker_abs).unwrap(),
            results = serde_json::to_string(&results_abs).unwrap(),
        );
        write(&root, ".bee/cells/leaky.json", &leaky_cell);

        let st = build_state();
        let project = register(&st, &root, "detail-security");
        let app = router(st);

        let cell_resp = get(app.clone(), &format!("/p/{}/_bee/cell/leaky-cell", project.id)).await;
        assert_eq!(cell_resp.status(), StatusCode::OK);
        let cell_body = body_string(cell_resp).await;

        assert!(!cell_body.contains(&root_str), "cell page leaked the fixture root: {cell_body}");
        assert!(
            !cell_body.contains(&outside_abs),
            "cell page leaked an absolute path outside the fixture root: {cell_body}"
        );
        assert!(!cell_body.contains(&worker_abs), "cell page leaked the absolute worker path: {cell_body}");
        assert!(
            !cell_body.contains(&inside_abs),
            "cell page leaked the in-root absolute path verbatim: {cell_body}"
        );
        assert!(!cell_body.contains(&results_abs), "cell page leaked the absolute results path: {cell_body}");
        assert!(cell_body.contains("src/inside.rs"), "{cell_body}");

        let feature_resp = get(app, &format!("/p/{}/_bee/feature/leaky-feature", project.id)).await;
        assert_eq!(feature_resp.status(), StatusCode::OK);
        let feature_body = body_string(feature_resp).await;

        assert!(!feature_body.contains(&root_str), "feature page leaked the fixture root: {feature_body}");
        assert!(
            !feature_body.contains(&outside_abs),
            "feature page leaked an absolute path outside the fixture root: {feature_body}"
        );
        assert!(
            !feature_body.contains(&worker_abs),
            "feature page leaked the absolute worker path: {feature_body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only) The detail routes read the same live `.bee/cells/*.json`
    /// tree as the board (D4) — the fixture tree must stay byte-identical
    /// before and after both requests.
    #[tokio::test]
    async fn detail_routes_never_write_the_fixtures_bee_tree() {
        let root = fresh_root("detail-read-only");
        write(&root, "README.md", "# hi");
        write(
            &root,
            ".bee/cells/a.json",
            &full_cell_json("ro-cell", "ro-feature", "capped", &[], &[]),
        );

        let st = build_state();
        let project = register(&st, &root, "detail-read-only");
        let before = snapshot_tree(&root);

        let app = router(st);
        let _ = get(app.clone(), &format!("/p/{}/_bee/cell/ro-cell", project.id)).await;
        let _ = get(app, &format!("/p/{}/_bee/feature/ro-feature", project.id)).await;

        let after = snapshot_tree(&root);
        assert_eq!(before, after, ".bee/ tree changed after a request");

        std::fs::remove_dir_all(&root).ok();
    }

    // ── bee-board-ux-3: "running now" section ──────────────────────────

    fn state_json_with_workers(workers_json: &str) -> String {
        format!(r#"{{"phase":"exploring","feature":"demo","mode":"standard","workers":[{workers_json}]}}"#)
    }

    fn worker_json(nickname: &str, cell: &str, tier: &str, status: &str) -> String {
        format!(r#"{{"nickname":"{nickname}","cell":"{cell}","tier":"{tier}","status":"{status}"}}"#)
    }

    /// (happy) A worker names a cell the store already calls `claimed`, and
    /// a session sharing the worker's nickname is live: the running section
    /// must show that worker's nickname, and it must link the cell it names
    /// to that cell's own detail page.
    #[tokio::test]
    async fn running_worker_with_live_session_links_to_its_cell_detail_page() {
        let root = fresh_root("running-happy");
        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "claimed", &[], "w1"));
        write(
            &root,
            ".bee/state.json",
            &state_json_with_workers(&worker_json("kf1-worker", "kf-1", "generation", "running")),
        );
        write(
            &root,
            ".bee/sessions/kf1-worker.json",
            &session_json("kf1-worker", &rfc3339_minutes_ago(1), "/home/x/t.jsonl", "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "running-happy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Running now"), "{body}");
        assert!(body.contains("kf1-worker"), "worker nickname must appear: {body}");
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/cell/kf-1\"", project.id)),
            "the named cell must link to its own detail page: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) A worker names a cell the store still calls `open` (the
    /// exact shape reported live: a claim never made it into the cell
    /// file). The running section must state that disagreement explicitly
    /// rather than hiding it.
    #[tokio::test]
    async fn running_worker_on_still_open_cell_shows_discrepancy_note() {
        let root = fresh_root("running-discrepancy");
        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "open", &[], "w1"));
        write(
            &root,
            ".bee/state.json",
            &state_json_with_workers(&worker_json("kf1-worker", "kf-1", "generation", "running")),
        );
        write(
            &root,
            ".bee/sessions/kf1-worker.json",
            &session_json("kf1-worker", &rfc3339_minutes_ago(1), "/home/x/t.jsonl", "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "running-discrepancy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("store still calls kf-1 open"),
            "the page must say the store still calls this cell open: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) The presence of a worker naming a still-open cell must never
    /// move that cell out of the Waiting bucket (D7 stays a pure function
    /// of cell status).
    #[tokio::test]
    async fn d7_buckets_unchanged_by_worker_presence() {
        let root = fresh_root("running-buckets-untouched");
        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "open", &[], "w1"));
        write(
            &root,
            ".bee/state.json",
            &state_json_with_workers(&worker_json("kf1-worker", "kf-1", "generation", "running")),
        );
        write(
            &root,
            ".bee/sessions/kf1-worker.json",
            &session_json("kf1-worker", &rfc3339_minutes_ago(1), "/home/x/t.jsonl", "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "running-buckets-untouched");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("data-bucket=\"waiting\" data-count=\"1\""),
            "an open cell must stay in Waiting even though a live worker names it: {body}"
        );
        assert!(
            body.contains("data-bucket=\"doing\" data-count=\"0\""),
            "worker data must never move a cell into Doing: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) No workers and no live session: one quiet line, not an empty
    /// bordered panel.
    #[tokio::test]
    async fn no_workers_and_no_live_session_renders_quiet_line() {
        let root = fresh_root("running-quiet");
        write(&root, ".bee/cells/a.json", &cell_json("a", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "running-quiet");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Nothing running right now."), "{body}");
        assert!(
            !body.contains("class=\"fg-card bee-panel bee-running\""),
            "the quiet empty state must not render an empty running panel: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A worker names a cell id that does not exist anywhere in the
    /// live cell store — it must be flagged, not silently dropped, and the
    /// page must still render.
    #[tokio::test]
    async fn worker_naming_nonexistent_cell_is_flagged_and_page_still_renders() {
        let root = fresh_root("running-ghost-cell");
        write(
            &root,
            ".bee/state.json",
            &state_json_with_workers(&worker_json("ghost-worker", "does-not-exist", "generation", "running")),
        );
        write(
            &root,
            ".bee/sessions/ghost-worker.json",
            &session_json("ghost-worker", &rfc3339_minutes_ago(1), "/home/x/t.jsonl", "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "running-ghost-cell");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("ghost-worker"), "{body}");
        assert!(
            body.contains("store has no cell named does-not-exist"),
            "a worker naming an unknown cell must be flagged, not dropped: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A worker whose matching session has gone stale must not be
    /// presented as running.
    #[tokio::test]
    async fn worker_with_stale_session_not_presented_as_running() {
        let root = fresh_root("running-stale-session");
        write(&root, ".bee/cells/kl-1.json", &cell_json("kl-1", "claimed", &[], "w1"));
        write(
            &root,
            ".bee/state.json",
            &state_json_with_workers(&worker_json("kl1-worker", "kl-1", "generation", "running")),
        );
        // 2 hours old: stale (session_live threshold is 30 minutes).
        write(
            &root,
            ".bee/sessions/kl1-worker.json",
            &session_json("kl1-worker", &rfc3339_minutes_ago(120), "/home/x/t.jsonl", "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "running-stale-session");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("Nothing running right now."),
            "a worker backed only by a stale session must not be presented as running: {body}"
        );
        // The existing Sessions panel legitimately still lists a stale
        // session (unrelated pre-existing behavior); what must be absent is
        // any *running-section* worker card for it — the running-panel
        // markup itself must not appear at all here.
        assert!(
            !body.contains("class=\"fg-card bee-panel bee-running\""),
            "a stale-session worker must not produce a running panel: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (security) The running section must leak no transcript path, no
    /// absolute path and no occurrence of the fixture root — named against
    /// the fixture's own root and `Path::is_absolute`, never a production
    /// literal (`docs/history/learnings/20260805-toothless-security-assertions.md`).
    #[tokio::test]
    async fn running_section_leaks_no_absolute_path_or_transcript() {
        let root = fresh_root("running-security");
        let root_str = root.to_string_lossy().into_owned();
        let transcript_abs = root.join(".bee/sessions/kf1-worker.json").to_string_lossy().into_owned();

        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "claimed", &[], "w1"));
        write(
            &root,
            ".bee/state.json",
            &state_json_with_workers(&worker_json("kf1-worker", "kf-1", "generation", "running")),
        );
        write(
            &root,
            ".bee/sessions/kf1-worker.json",
            &session_json("kf1-worker", &rfc3339_minutes_ago(1), &transcript_abs, "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "running-security");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(!body.contains(&transcript_abs), "transcript path leaked into the body: {body}");
        assert!(!body.contains(&root_str), "response body leaked the fixture root: {body}");
        assert!(
            body.contains("kf1-worker"),
            "the security assertions above must exercise the running section, not skip it: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only) Reading the board with live workers/sessions present must
    /// not touch the fixture's `.bee/` tree (D4).
    #[tokio::test]
    async fn running_section_read_never_writes_the_fixtures_bee_tree() {
        let root = fresh_root("running-read-only");
        write(&root, "README.md", "# hi");
        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "claimed", &[], "w1"));
        write(
            &root,
            ".bee/state.json",
            &state_json_with_workers(&worker_json("kf1-worker", "kf-1", "generation", "running")),
        );
        write(
            &root,
            ".bee/sessions/kf1-worker.json",
            &session_json("kf1-worker", &rfc3339_minutes_ago(1), "/home/x/t.jsonl", "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "running-read-only");
        let before = snapshot_tree(&root);

        let _ = get(router(st), &format!("/p/{}/_bee", project.id)).await;

        let after = snapshot_tree(&root);
        assert_eq!(before, after, ".bee/ tree changed after a request");

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bee-board-ux-4: each granted worktree, its own lifecycle record ---

    /// Sibling worktree directories sit beside `fresh_root`'s temp parent —
    /// the exact shape `mdview_core::bee::resolve_worktree` expects: `<temp
    /// dir>/<id>/.bee/...`.
    fn worktree_sibling_root(id: &str) -> PathBuf {
        std::env::temp_dir().join(id)
    }

    fn make_worktree_sibling(id: &str) -> PathBuf {
        let dir = worktree_sibling_root(id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn grants_json(ids: &[&str]) -> String {
        let entries: String = ids.iter().map(|id| format!("\"{id}\": true")).collect::<Vec<_>>().join(",");
        format!("{{{entries}}}")
    }

    /// (happy) A fixture with two granted worktrees renders each with its
    /// own feature, phase and branch.
    #[tokio::test]
    async fn worktree_section_shows_each_granted_worktree_with_own_feature_phase_branch() {
        let root = fresh_root("wt-two");
        let alpha = make_worktree_sibling("bee-board-ux-4-srv-wt-alpha");
        let beta = make_worktree_sibling("bee-board-ux-4-srv-wt-beta");
        write(&alpha, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-alpha","mode":"standard"}"#);
        write(&beta, ".bee/state.json", r#"{"phase":"planning","feature":"feat-beta","mode":"small"}"#);

        write(
            &root,
            ".bee/runtime/worktree-grants.json",
            &grants_json(&["bee-board-ux-4-srv-wt-alpha", "bee-board-ux-4-srv-wt-beta"]),
        );
        write(
            &root,
            ".bee/runtime/workspaces/alpha.json",
            &workspace_json("bee-board-ux-4-srv-wt-alpha", &alpha.to_string_lossy(), "wt/alpha", &[]),
        );
        write(
            &root,
            ".bee/runtime/workspaces/beta.json",
            &workspace_json("bee-board-ux-4-srv-wt-beta", &beta.to_string_lossy(), "wt/beta", &[]),
        );

        let st = build_state();
        let project = register(&st, &root, "wt-two");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("feature: feat-alpha"), "{body}");
        assert!(body.contains("phase: swarming"), "{body}");
        assert!(body.contains("branch: wt/alpha"), "{body}");
        assert!(body.contains("feature: feat-beta"), "{body}");
        assert!(body.contains("phase: planning"), "{body}");
        assert!(body.contains("branch: wt/beta"), "{body}");

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&alpha).ok();
        std::fs::remove_dir_all(&beta).ok();
    }

    /// (happy) A worktree holding a live session is presented as live with a
    /// relative heartbeat age and sorts ahead of one that is not.
    #[tokio::test]
    async fn worktree_with_live_session_sorts_before_quiet_and_shows_heartbeat_age() {
        let root = fresh_root("wt-live-sort");
        let live = make_worktree_sibling("bee-board-ux-4-srv-wt-live");
        let quiet = make_worktree_sibling("bee-board-ux-4-srv-wt-quiet");
        write(&live, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-live","mode":"standard"}"#);
        write(&quiet, ".bee/state.json", r#"{"phase":"idle","feature":"feat-quiet","mode":"standard"}"#);
        write(
            &live,
            ".bee/sessions/s1.json",
            &session_json("s1", &rfc3339_minutes_ago(2), "/home/x/t.jsonl", "main", "startup"),
        );

        write(
            &root,
            ".bee/runtime/worktree-grants.json",
            // "quiet" listed first in the source file — the sort must still
            // put the live one ahead regardless of grant order.
            &grants_json(&["bee-board-ux-4-srv-wt-quiet", "bee-board-ux-4-srv-wt-live"]),
        );

        let st = build_state();
        let project = register(&st, &root, "wt-live-sort");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        let live_pos = body.find("feat-live").expect("live worktree must render");
        let quiet_pos = body.find("feat-quiet").expect("quiet worktree must render");
        assert!(live_pos < quiet_pos, "live worktree must render before the quiet one: {body}");
        assert!(
            body.contains("fg-chip--success\">live<"),
            "the live worktree must carry a live chip: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&live).ok();
        std::fs::remove_dir_all(&quiet).ok();
    }

    /// (happy) Worktree cell files present in the fixture change NEITHER the
    /// four D7 bucket counts NOR the shipped set — the regression that
    /// motivated this cell.
    #[tokio::test]
    async fn worktree_cell_files_do_not_change_buckets_or_shipped_set() {
        let root = fresh_root("wt-no-cell-merge");
        write(&root, ".bee/cells/a.json", &cell_json("c-open", "open", &[], "w1"));

        let sibling = make_worktree_sibling("bee-board-ux-4-srv-wt-cells");
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"ghost-feature","mode":"standard"}"#);
        // A capped cell sitting only in the worktree's own store. If this
        // were ever merged into the main snapshot it would move into the
        // Done bucket and appear as an extra shipped feature.
        write(
            &sibling,
            ".bee/cells/ghost.json",
            &feature_cell_json("ghost-1", "ghost-feature", "capped", Some(&rfc3339_minutes_ago(60)), Some(&rfc3339_minutes_ago(1))),
        );

        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-srv-wt-cells"]));

        let st = build_state();
        let project = register(&st, &root, "wt-no-cell-merge");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-bucket=\"waiting\" data-count=\"1\""), "{body}");
        assert!(body.contains("data-bucket=\"doing\" data-count=\"0\""), "{body}");
        assert!(
            body.contains("data-bucket=\"done\" data-count=\"0\""),
            "a worktree's own capped cell must never move this project's Done bucket: {body}"
        );
        assert!(
            !body.contains("ghost-1"),
            "the worktree's own cell id must never render on this project's board: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }

    fn feature_cell_json(
        id: &str,
        feature: &str,
        status: &str,
        claimed_at: Option<&str>,
        capped_at: Option<&str>,
    ) -> String {
        let claimed_json = claimed_at.map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string());
        let capped_json = capped_at.map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string());
        format!(
            r#"{{
                "id": "{id}",
                "feature": "{feature}",
                "lane": "standard",
                "title": "Cell {id}",
                "action": "do the thing",
                "verify": "cargo test",
                "files": [],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {{}},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "{status}",
                "tier": "generation",
                "trace": {{"worker": "w1", "claimed_at": {claimed_json}, "capped_at": {capped_json}}}
            }}"#
        )
    }

    /// (edge) A granted id whose directory does not exist is reported as
    /// unresolved and the page still renders.
    #[tokio::test]
    async fn worktree_directory_missing_is_unresolved_and_page_still_renders() {
        let root = fresh_root("wt-dir-missing");
        std::fs::remove_dir_all(worktree_sibling_root("bee-board-ux-4-srv-wt-ghost-dir")).ok();
        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-srv-wt-ghost-dir"]));

        let st = build_state();
        let project = register(&st, &root, "wt-dir-missing");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("bee-board-ux-4-srv-wt-ghost-dir"), "{body}");
        assert!(body.contains("unresolved"), "a dangling grant must be marked unresolved: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A granted id whose state.json is malformed is reported as
    /// unresolved, not fatal.
    #[tokio::test]
    async fn worktree_state_json_malformed_is_unresolved_not_fatal() {
        let root = fresh_root("wt-state-malformed");
        let sibling = make_worktree_sibling("bee-board-ux-4-srv-wt-malformed");
        write(&sibling, ".bee/state.json", "{ not valid json");
        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-srv-wt-malformed"]));

        let st = build_state();
        let project = register(&st, &root, "wt-state-malformed");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("bee-board-ux-4-srv-wt-malformed"), "{body}");
        assert!(body.contains("unresolved"), "a malformed state.json must be marked unresolved, not crash the page: {body}");

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }

    /// (edge) A project with no grants file renders the quiet one-line
    /// state, not an empty panel.
    #[tokio::test]
    async fn no_grants_file_renders_quiet_line_not_empty_panel() {
        let root = fresh_root("wt-no-grants");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);

        let st = build_state();
        let project = register(&st, &root, "wt-no-grants");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("No worktrees granted."), "{body}");
        assert!(
            !body.contains("class=\"fg-card bee-panel bee-worktrees\""),
            "no grants must not render an empty worktrees panel: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (security) The body contains no absolute worktree root, no transcript
    /// path and no occurrence of the fixture root — named against the
    /// fixture's own root and `Path::is_absolute`, never the literal
    /// `/home/` (`docs/history/learnings/20260805-toothless-security-assertions.md`).
    #[tokio::test]
    async fn worktree_section_leaks_no_absolute_path_or_fixture_root() {
        let root = fresh_root("wt-security");
        let root_str = root.to_string_lossy().into_owned();
        let sibling = make_worktree_sibling("bee-board-ux-4-srv-wt-security");
        let sibling_str = sibling.to_string_lossy().into_owned();
        let transcript_abs = sibling.join(".bee/sessions/s1.json").to_string_lossy().into_owned();

        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-sec","mode":"standard"}"#);
        write(
            &sibling,
            ".bee/sessions/s1.json",
            &session_json("s1", &rfc3339_minutes_ago(2), &transcript_abs, "main", "startup"),
        );

        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-srv-wt-security"]));
        write(
            &root,
            ".bee/runtime/workspaces/w1.json",
            &workspace_json("bee-board-ux-4-srv-wt-security", &sibling_str, "wt/security", &[]),
        );

        let st = build_state();
        let project = register(&st, &root, "wt-security");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(!body.contains(&root_str), "response body leaked the fixture root: {body}");
        assert!(!body.contains(&sibling_str), "response body leaked the worktree's own absolute sibling root: {body}");
        assert!(!body.contains(&transcript_abs), "response body leaked a transcript path: {body}");
        assert!(
            body.contains("feat-sec") && body.contains("bee-board-ux-4-srv-wt-security"),
            "the security assertions above must exercise the worktree section, not skip it: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }

    /// (read-only) Both the project's and the worktree's own `.bee/` tree
    /// are byte-identical before and after the request (D4).
    #[tokio::test]
    async fn worktree_read_never_writes_the_project_or_sibling_bee_tree() {
        let root = fresh_root("wt-read-only");
        let sibling = make_worktree_sibling("bee-board-ux-4-srv-wt-read-only");
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"feat-ro","mode":"standard"}"#);
        write(
            &sibling,
            ".bee/sessions/s1.json",
            &session_json("s1", &rfc3339_minutes_ago(2), "/home/x/t.jsonl", "main", "startup"),
        );
        write(&root, ".bee/runtime/worktree-grants.json", &grants_json(&["bee-board-ux-4-srv-wt-read-only"]));

        let st = build_state();
        let project = register(&st, &root, "wt-read-only");
        let before_root = snapshot_tree(&root);
        let before_sibling = snapshot_tree(&sibling);

        let _ = get(router(st), &format!("/p/{}/_bee", project.id)).await;

        let after_root = snapshot_tree(&root);
        let after_sibling = snapshot_tree(&sibling);
        assert_eq!(before_root, after_root, ".bee/ tree changed after a request");
        assert_eq!(before_sibling, after_sibling, "the worktree's own .bee/ tree changed after a request");

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&sibling).ok();
    }

    /// The seam this cell exists to add: `/settings` and `/api/config` must
    /// read through `AppState::config_data_dir` instead of the process-global
    /// `~/.mdview`, so a route-level test can point at a temp dir and never
    /// touch the developer's real config file (agent-terminal-1, E0/S1).
    #[tokio::test]
    async fn settings_routes_read_through_the_injected_data_dir_not_real_home() {
        let dir = fresh_root("settings-override-read");
        // A distinctive value that only exists in the override dir's config,
        // never written to the real ~/.mdview by this test.
        let mut cfg = Config::default();
        cfg.server.port = 47201;
        cfg.save_to(&dir.join("config.toml")).unwrap();

        let real_config_path = mdview_core::config::config_path();
        let real_before = std::fs::read(&real_config_path).ok();

        let mut st = build_state();
        st.config_data_dir = Some(dir.clone());

        let resp = get(router(st.clone()), "/settings").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("47201"),
            "settings page did not reflect the overridden data dir's config"
        );

        let resp = get(router(st), "/api/config").await;
        let body = body_string(resp).await;
        assert!(
            body.contains("47201"),
            "/api/config did not reflect the overridden data dir's config"
        );

        let real_after = std::fs::read(&real_config_path).ok();
        assert_eq!(
            real_before, real_after,
            "the real ~/.mdview/config.toml was read or written by a route test"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `update_config` must save through the same injected path — a POST from
    /// a route test must never land in the real `~/.mdview/config.toml`.
    #[tokio::test]
    async fn update_config_writes_only_to_the_injected_data_dir() {
        let dir = fresh_root("settings-override-write");

        let real_config_path = mdview_core::config::config_path();
        let real_before = std::fs::read(&real_config_path).ok();

        let mut st = build_state();
        st.config_data_dir = Some(dir.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/api/config")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("port=58311"))
            .unwrap();
        let resp = router(st).oneshot(req).await.unwrap();
        assert!(
            resp.status().is_redirection(),
            "update_config should redirect back to /settings, got {}",
            resp.status()
        );

        let saved = Config::load_from(&dir.join("config.toml"));
        assert_eq!(
            saved.server.port, 58311,
            "update_config did not write the overridden data dir's config file"
        );

        let real_after = std::fs::read(&real_config_path).ok();
        assert_eq!(
            real_before, real_after,
            "the real ~/.mdview/config.toml was written by update_config through a route test"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D10/P1: the token is generated on the settings page but never lives in
    /// `Config`, so `GET /api/config` must never carry it — before or after
    /// generation (agent-terminal-4, E2).
    #[tokio::test]
    async fn api_config_never_contains_the_token_value_before_or_after_generation() {
        let dir = fresh_root("terminal-token-not-in-api-config");
        let st = build_state_with_dir(&dir);

        let resp = get(router(st.clone()), "/api/config").await;
        let body_before = body_string(resp).await;
        assert!(
            !body_before.to_lowercase().contains("token"),
            "GET /api/config mentioned a token before one was ever generated: {body_before}"
        );

        let (full_token, _cookie) = rotate_token(router(st.clone())).await;

        let resp = get(router(st), "/api/config").await;
        let body_after = body_string(resp).await;
        assert!(
            !body_after.contains(&full_token),
            "GET /api/config leaked the generated token value: {body_after}"
        );
        assert!(
            !body_after.to_lowercase().contains("token"),
            "GET /api/config gained a token-shaped field after generation: {body_after}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// P2: the settings page shows the token in full only in the response
    /// that generated or rotated it; every later render shows only its last
    /// four characters.
    #[tokio::test]
    async fn settings_page_reveals_the_token_in_full_once_then_masks_it() {
        let dir = fresh_root("terminal-token-reveal-once");
        let st = build_state_with_dir(&dir);

        let (full_token, cookie) = rotate_token(router(st.clone())).await;
        assert_eq!(full_token.len(), 64, "unexpected token shape: {full_token}");
        let last_four = &full_token[full_token.len() - 4..];

        // A second settings render (any request, session or not) must never
        // carry the full value again — only its last four characters.
        let resp = get(router(st.clone()), "/settings").await;
        let body = body_string(resp).await;
        assert!(
            !body.contains(&full_token),
            "a second /settings render leaked the full token: {body}"
        );
        assert!(
            body.contains(last_four),
            "a second /settings render dropped the masked token entirely: {body}"
        );

        // The session minted by rotation is real and usable, proving the
        // masking above isn't hiding a second reveal path through it either.
        assert!(!cookie.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// P3: `POST /api/config` is unauthenticated (D4 leaves it that way), so
    /// it must never be able to move a D7 switch — a supervisor field there
    /// would let any LAN visitor make mdview spawn a process.
    #[tokio::test]
    async fn post_api_config_with_terminal_fields_leaves_every_switch_unchanged() {
        let dir = fresh_root("terminal-switches-not-via-api-config");
        let st = build_state_with_dir(&dir);

        let req = Request::builder()
            .method("POST")
            .uri("/api/config")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(
                "port=58312&enabled=on&supervisor_enabled=on&notify_enabled=on",
            ))
            .unwrap();
        let resp = router(st).oneshot(req).await.unwrap();
        assert!(resp.status().is_redirection());

        let saved = Config::load_from(&dir.join("config.toml"));
        assert_eq!(saved.server.port, 58312, "the legitimate field was not saved");
        assert!(
            !saved.terminal.enabled,
            "POST /api/config flipped the terminal enable switch"
        );
        assert!(
            !saved.terminal.supervisor_enabled,
            "POST /api/config flipped the supervisor switch"
        );
        assert!(
            !saved.terminal.notify_enabled,
            "POST /api/config flipped the notify switch"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// P3: the switches can be changed only by a request carrying a valid
    /// terminal session — no session, and a stale/unknown session, both
    /// leave every switch untouched; a session minted by rotation succeeds.
    #[tokio::test]
    async fn terminal_switches_require_a_valid_terminal_session() {
        let dir = fresh_root("terminal-switches-gated");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());

        let switches_req = |cookie: Option<&str>| {
            let mut b = Request::builder()
                .method("POST")
                .uri("/api/terminal-config")
                .header("content-type", "application/x-www-form-urlencoded");
            if let Some(c) = cookie {
                b = b.header(header::COOKIE, c.to_string());
            }
            b.body(Body::from("enabled=on&supervisor_enabled=on&notify_enabled=on"))
                .unwrap()
        };

        // No session at all.
        let resp = app.clone().oneshot(switches_req(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // An unknown/stale session cookie.
        let resp = app
            .clone()
            .oneshot(switches_req(Some("mdview_terminal_session=not-a-real-session")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let cfg_after_refusals = Config::load_from(&dir.join("config.toml"));
        assert!(!cfg_after_refusals.terminal.enabled);
        assert!(!cfg_after_refusals.terminal.supervisor_enabled);
        assert!(!cfg_after_refusals.terminal.notify_enabled);

        // A real session, minted by generating the token, succeeds.
        let (_full_token, cookie) = rotate_token(app.clone()).await;
        let resp = app
            .clone()
            .oneshot(switches_req(Some(&cookie)))
            .await
            .unwrap();
        assert!(
            resp.status().is_redirection(),
            "a valid terminal session could not save the switches, got {}",
            resp.status()
        );

        let saved = Config::load_from(&dir.join("config.toml"));
        assert!(saved.terminal.enabled);
        assert!(saved.terminal.supervisor_enabled);
        assert!(saved.terminal.notify_enabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A config that has never seen the terminal section at all — the exact
    /// shape of every install predating this cell — must resolve every
    /// switch to off, proven through the same route a browser hits.
    #[tokio::test]
    async fn api_config_shows_every_terminal_switch_off_by_default() {
        let dir = fresh_root("terminal-switches-default-off");
        let st = build_state_with_dir(&dir);

        let resp = get(router(st), "/api/config").await;
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["terminal"]["enabled"], serde_json::json!(false));
        assert_eq!(json["terminal"]["supervisor_enabled"], serde_json::json!(false));
        assert_eq!(json["terminal"]["notify_enabled"], serde_json::json!(false));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// POSTs `/settings/terminal/token`, returning the full token revealed in
    /// that one response plus the session cookie value minted alongside it —
    /// the shared setup every gated-switch test needs to get past the gate.
    async fn rotate_token(app: Router) -> (String, String) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/terminal/token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let set_cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .expect("rotate response must set the terminal session cookie")
            .to_str()
            .unwrap()
            .to_string();
        let cookie = set_cookie.split(';').next().unwrap().to_string();
        let body = body_string(resp).await;
        let marker = "it will not be shown again: <code>";
        let start = body.find(marker).expect("full token banner missing") + marker.len();
        let rest = &body[start..];
        let end = rest.find("</code>").expect("full token banner unterminated");
        (rest[..end].to_string(), cookie)
    }
}
