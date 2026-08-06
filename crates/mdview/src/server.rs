//! Axum daemon: routes, live-reload WebSocket, filesystem watcher.

use crate::herdr::{self, Herdr};
use crate::runtime::{self, DaemonInfo};
use crate::terminal_auth::{self, HasTerminalAuth, TerminalAuth};
use crate::views;
use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Form, Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{any, get, post},
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
    /// The herdr client (agent-terminal-2): a real `SocketHerdr` in
    /// production, a `FakeHerdr` in every test — `Arc<dyn Herdr>` so a route
    /// test can swap in a socket-free double without touching this field's
    /// type. Every terminal route reaches herdr only through this handle,
    /// never by constructing its own client.
    pub herdr: Arc<dyn Herdr>,
    /// Overrides `mdview_core::transcript`'s default Claude Code projects
    /// root (`terminal_transcript`) so a route-level test can point
    /// transcript I/O at a scratch dir instead of the developer's real
    /// `~/.claude/projects` — the same seam `config_data_dir` gives the
    /// settings routes over `~/.mdview`. `None` in production: the
    /// transcript reader then resolves the root exactly where Claude Code
    /// itself writes it.
    pub transcript_root: Option<PathBuf>,
    /// D7's live background manager (agent-terminal-18): reconciled against
    /// the D7 switches at startup and on every `update_terminal_config`
    /// write, so flipping a switch takes effect without a restart. One
    /// instance per `AppState`, shared (via its own internal `Arc`-free
    /// interior mutability) across every clone the way `terminal_auth` is.
    pub terminal_background: Arc<crate::TerminalBackground>,
    /// The D7/D9 notification outbox (agent-terminal-17): opened once at
    /// startup — in-memory for every route test, so no test ever touches
    /// the real `~/.mdview/notify.sqlite` — and handed to
    /// `terminal_background` on every reconcile rather than reopened per
    /// toggle.
    pub notify_store: Arc<mdview_core::notify_store::NotifyStore>,
}

impl HasTerminalAuth for AppState {
    fn terminal_auth(&self) -> &TerminalAuth {
        &self.terminal_auth
    }
}

/// D7/D9 outbox (agent-terminal-22): the only code path that ever reads or
/// writes this store is `TerminalBackground::reconcile_notify` (the notify
/// switch); the supervisor switch never touches it. So the real sqlite file
/// — and the `.sqlite-wal`/`.sqlite-shm` sidecars `NotifyStore::open`'s WAL
/// mode creates alongside it — is only opened when `cfg.notify_enabled` is
/// already true. A config with the switch off gets an in-memory store
/// instead: the exact same `NotifyStore` type every other code path already
/// expects, so a markdown-only install that never flips the switch never
/// gains a database file it didn't ask for.
///
/// A failure to open the real sqlite file (permissions, disk full, …) must
/// never crash a daemon whose primary job is serving markdown — fall back to
/// an in-memory outbox rather than propagate the error, matching every other
/// "degrade rather than fail" seam this feature already has (D6's herdr-down
/// state, the boundary's fail-closed empty pane list).
fn open_notify_store(
    cfg: &mdview_core::config::TerminalConfig,
    override_dir: Option<&std::path::Path>,
) -> mdview_core::notify_store::NotifyStore {
    if !cfg.notify_enabled {
        return mdview_core::notify_store::NotifyStore::open_in_memory()
            .expect("in-memory sqlite always opens");
    }
    let notify_store_path = mdview_core::config::notify_store_path_override(override_dir);
    mdview_core::notify_store::NotifyStore::open(&notify_store_path).unwrap_or_else(|e| {
        tracing::warn!("notify outbox open failed ({e}); using an in-memory outbox");
        mdview_core::notify_store::NotifyStore::open_in_memory()
            .expect("in-memory sqlite always opens")
    })
}

/// Start the daemon: watcher + HTTP server. Blocks until shutdown.
pub async fn serve() -> Result<()> {
    let engine = Arc::new(runtime::build_engine()?);
    let (reload_tx, _) = broadcast::channel::<String>(32);
    let highlight_css = Arc::new(build_highlight_css(&engine));

    // Best-effort: an unresolvable default socket path (platform/env
    // oddity) never blocks the daemon from starting — it just means every
    // `snapshot()` call fails with `Unavailable`, which the terminal route
    // already renders as the D6 "herdr is not running" state rather than a
    // raw error or a crash.
    let herdr_socket_path = herdr::socket::default_socket_path()
        .unwrap_or_else(|_| PathBuf::from("/nonexistent/herdr.sock"));
    let notify_store = Arc::new(open_notify_store(&engine.config.terminal, None));
    let state = AppState {
        engine: engine.clone(),
        reload_tx: reload_tx.clone(),
        highlight_css,
        config_data_dir: None,
        terminal_auth: TerminalAuth::new(None),
        herdr: Arc::new(herdr::socket::SocketHerdr::new(herdr_socket_path)),
        transcript_root: None,
        terminal_background: Arc::new(crate::TerminalBackground::new()),
        notify_store,
    };

    // D7: reconcile the live background against whatever the config already
    // says, once at startup — a config that already had a switch on before
    // this restart must keep behaving as if it does, not silently go dark
    // until the operator re-saves the settings page.
    let telegram = telegram_credentials(&engine.config.terminal, state.config_data_dir.as_deref());
    state.terminal_background.reconcile(
        &engine.config.terminal,
        state.herdr.clone(),
        state.notify_store.clone(),
        telegram,
    );

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
             authentication outside the agent terminal, transcript and \
             agent-creation routes — anyone who can reach this port can read \
             every indexed file and each project's filesystem path. Bind \
             127.0.0.1 unless you intend LAN exposure.",
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
        // agent-terminal-11: was mounted with `.post(...)`, the same
        // method-mismatch-oracle gap every other route in this family
        // closes with `any(...)` + `MethodGate<Post>` — a `GET` here used
        // to answer `405 Allow: POST`, distinguishable from an unrouted
        // path without ever checking a session or a token.
        .route("/settings/terminal/token", any(rotate_terminal_token))
        // agent-terminal-8: the only route that ever turns a presented token
        // into a session — see `login_terminal`. `any(...)` + `MethodGate<Post>`
        // (inside the handler) for the same method-mismatch-oracle reason as
        // every other route in this family: `.post(...)` would let an
        // unauthenticated `GET` here answer `405 Allow: POST` instead of the
        // same opaque 404 an unrouted path returns.
        .route("/settings/terminal/login", any(login_terminal))
        // Carry-over from agent-terminal-4: mounted with `.post(...)` before
        // `MethodGate` existed, which let a `GET` here answer `405 Allow:
        // POST` — distinguishable from an unrouted path without a token ever
        // being checked. `any(...)` + `MethodGate<Post>` (inside the
        // handler) closes that oracle the same way every other gated
        // terminal route does.
        .route("/api/terminal-config", any(update_terminal_config))
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
        // Gated (D4): `any(...)` + `MethodGate<Get>` inside the handler, not
        // `.get(...)`, for the same method-mismatch-oracle reason as above.
        .route("/p/:id/_terminal", any(terminal_page))
        // agent-terminal-16 (D9): the Transcript tab — a second tab beside
        // Terminal, not a toggle inside its frame. Same gate shape as the
        // page above (`any(...)` + `MethodGate<Get>` inside the handler).
        .route("/p/:id/_transcript", any(transcript_page))
        // agent-terminal-6: one pane's polled screen, same gate shape as the
        // page above (`any(...)` + `MethodGate<Get>` inside the handler).
        .route("/p/:id/_terminal/:pane_id/screen", any(terminal_screen))
        // agent-terminal-16 (D9): the gap-free activity channel beside the
        // screen above — same gate shape, same D2 containment boundary,
        // applied via `project_pane_cwd_in_boundary` rather than
        // `project_and_verify_pane_in_boundary` since this route needs the
        // pane's own cwd value, not just a membership check.
        .route("/p/:id/_terminal/:pane_id/transcript", any(terminal_transcript))
        // agent-terminal-9 (D3): the write side — free text and named keys
        // into a pane. Same `any(...)` + `MethodGate<Post>` shape as every
        // other gated terminal route, never `.post(...)`.
        .route("/p/:id/_terminal/:pane_id/input", any(terminal_input))
        .route("/p/:id/_terminal/:pane_id/keys", any(terminal_keys))
        // agent-terminal-13 (D8/P4): start a new pane or agent in this
        // project. Same `any(...)` + `MethodGate<Post>` shape as every
        // other gated terminal route, never `.post(...)` — and the same D2
        // containment boundary the routes above use, applied to the
        // destination workspace's own anchor rather than an already-listed
        // pane id, so a session can never start a process in a project it
        // is not looking at.
        .route("/p/:id/_terminal/create/pane", any(terminal_create_pane))
        .route("/p/:id/_terminal/create/agent", any(terminal_create_agent))
        // agent-terminal-10 (D5): the Unassigned group — panes under no
        // registered project's root. Deliberately mounted outside `/p/:id/`
        // (never `/p/unassigned/...`): a registered project's own slug can
        // legitimately be the literal string "unassigned" (`slug_from_root`
        // has no reserved-word exclusion), so nesting this under the
        // project path shape would make that real project's own terminal
        // route ambiguous with this group's route. Same `any(...)` +
        // `MethodGate` shape as every other gated terminal route, never
        // `.get(...)` / `.post(...)`.
        .route("/_terminal/unassigned", any(unassigned_terminal_page))
        .route(
            "/_terminal/unassigned/:pane_id/screen",
            any(unassigned_terminal_screen),
        )
        .route(
            "/_terminal/unassigned/:pane_id/input",
            any(unassigned_terminal_input),
        )
        .route(
            "/_terminal/unassigned/:pane_id/keys",
            any(unassigned_terminal_keys),
        )
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
            // D5/D4: presence only, never contents — this unauthenticated
            // route reads only the D7 switch (no herdr call, no session), so
            // it can never learn whether any pane is actually unassigned.
            let unassigned_visible = terminal_family_enabled(&st);
            Html(views::project_list_page(&with_counts, unassigned_visible)).into_response()
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
/// `root_path`: this route carries no authentication (it is outside the
/// agent terminal family, D4), so exposing each project's filesystem layout
/// over `/api/projects` leaks it to anyone who can reach the port (see the
/// non-loopback bind warning in `serve`).
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
    /// Set by `update_terminal_config` when `save_notify_credential` failed
    /// (agent-terminal-24) — a distinct query flag rather than overloading
    /// `saved`, so a failed credential write is never rendered as success.
    /// Only ever carries the literal `"1"`; never the credential itself
    /// (that would put the secret in the redirect target, which this cell's
    /// own prohibition forbids).
    notify_error: Option<String>,
}

async fn settings_page_handler(State(st): State<AppState>, Query(flag): Query<SavedFlag>) -> Response {
    // Read fresh from disk so the form reflects the last save (the running daemon
    // still uses its startup config until restarted — noted in the UI).
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    let token_view = current_token_view(&st);
    let notify_credential_view = current_notify_credential_view(&st);
    Html(views::settings_page(
        &cfg,
        flag.saved.is_some(),
        flag.notify_error.is_some(),
        token_view,
        notify_credential_view,
    ))
    .into_response()
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

/// The Telegram credential state `settings_page` renders — masked to the
/// last four characters, or "never saved". Unlike [`current_token_view`]
/// there is no reveal-once counterpart anywhere in this module: this is the
/// only view of the credential any response ever carries, including the one
/// immediately after `update_terminal_config` saves it.
fn current_notify_credential_view(st: &AppState) -> views::NotifyCredentialView {
    let cred_path =
        mdview_core::config::notify_credential_path_override(st.config_data_dir.as_deref());
    match mdview_core::config::masked_notify_credential(&cred_path) {
        Some(masked) => views::NotifyCredentialView::Masked(masked),
        None => views::NotifyCredentialView::NotConfigured,
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
/// four characters.
///
/// agent-terminal-8: this route no longer mints a session. It used to, on
/// the reasoning that reaching the (already-unauthenticated) settings
/// surface was itself sufficient proof — but that made
/// `curl -X POST /settings/terminal/token` a complete, credential-free login
/// bypass: a live session for the price of one POST, with `verify_and_mint`
/// (the only real token check in the product) never in the loop. Rotation
/// now only ever reveals the fresh token (P2); the caller logs in with it
/// like anyone else, through `login_terminal` below — the only function in
/// the product that ever mints a session from a presented credential.
///
/// Rotation itself is gated once a token exists: the very first call — no
/// token file on disk yet — is left open, because that is the genuine
/// first-run case setup depends on. Every later rotation requires the
/// caller to already hold a live terminal session; without that, any LAN
/// visitor could rotate at will and silently clear the legitimate user's
/// session (P5's "rotation cuts live sessions" turned into a denial of
/// service against a user who never asked to rotate). Gating on the current
/// session also means a second device can log in with a token it already
/// holds instead of being forced to rotate and kick the first device out —
/// the multi-device flow D3 needs.
async fn rotate_terminal_token(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if st.terminal_auth.is_configured() {
        let has_session = terminal_auth::session_cookie(&headers)
            .map(|sid| st.terminal_auth.session_valid(&sid))
            .unwrap_or(false);
        if !has_session {
            return terminal_auth::opaque_404();
        }
    }
    match st.terminal_auth.rotate() {
        Ok(full_token) => {
            let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
                st.config_data_dir.as_deref(),
            ));
            let notify_credential_view = current_notify_credential_view(&st);
            let html = views::settings_page(
                &cfg,
                false,
                false,
                views::TerminalTokenView::Full(full_token),
                notify_credential_view,
            );
            Html(html).into_response()
        }
        Err(e) => internal_error(&e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct LoginForm {
    token: String,
}

/// POST /settings/terminal/login (agent-terminal-8) — the only place in the
/// product a raw presented token is turned into a live session. Calls
/// `TerminalAuth::verify_and_mint`, the only function that ever compares a
/// presented token against the configured one; `None` (missing token file,
/// wrong value, or empty) answers the same opaque 404 every other terminal
/// auth failure does — never a 401/403 that would confirm the route exists.
/// `Some(session_id)` sets the session cookie and sends the caller back to
/// `/settings`, where the gated switches below now become reachable.
async fn login_terminal(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    State(st): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    match st.terminal_auth.verify_and_mint(&form.token) {
        Some(session_id) => (
            [(header::SET_COOKIE, terminal_auth::session_cookie_header(&session_id))],
            Redirect::to("/settings"),
        )
            .into_response(),
        None => terminal_auth::opaque_404(),
    }
}

#[derive(serde::Deserialize, Default)]
struct TerminalConfigForm {
    enabled: Option<String>,
    supervisor_enabled: Option<String>,
    notify_enabled: Option<String>,
    /// D7/D9 notification destination — a plain (non-secret) field, see
    /// `TerminalConfig::notify_chat_id`. Only overwrites the stored value
    /// when non-blank, matching every other optional field on this form
    /// (`update_config`'s host/exclude_patterns/etc): submitting the form
    /// with the field left as rendered never clobbers it.
    notify_chat_id: Option<String>,
    /// D7/D9 notification credential (the Telegram bot token). Write-only:
    /// this form never receives the current value back to redisplay it, so
    /// a blank submission always means "leave the saved credential alone",
    /// never "clear it".
    notify_telegram_token: Option<String>,
}

/// POST /api/terminal-config — the D7 switches (terminal enable, herdr
/// supervisor, Telegram notification). Per P3 this is deliberately its own
/// route rather than a field on `SettingsForm`/`update_config`:
/// `POST /api/config` is unauthenticated, so a supervisor switch reachable
/// there would let any LAN visitor make mdview spawn a process. `AuthSession`
/// requires a live terminal session (minted only by `login_terminal` above);
/// on any auth failure the request never reaches this handler at all —
/// `AuthSession`'s extractor short-circuits with the opaque 404 before the
/// switches are read, let alone changed.
async fn update_terminal_config(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    State(st): State<AppState>,
    _session: terminal_auth::AuthSession,
    Form(form): Form<TerminalConfigForm>,
) -> Response {
    let config_path = mdview_core::config::config_path_override(st.config_data_dir.as_deref());
    let mut cfg = mdview_core::Config::load_from(&config_path);
    cfg.terminal.enabled = form.enabled.is_some();
    cfg.terminal.supervisor_enabled = form.supervisor_enabled.is_some();
    cfg.terminal.notify_enabled = form.notify_enabled.is_some();
    if let Some(dest) = form
        .notify_chat_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        cfg.terminal.notify_chat_id = Some(dest.to_string());
    }
    // Per this cell's own rule (mirroring P1): the credential is never a
    // `Config` field, so it is written to its own owner-only file, not into
    // `cfg` — a blank submission leaves whatever is already on disk alone.
    let cred_path = mdview_core::config::notify_credential_path_override(st.config_data_dir.as_deref());
    // agent-terminal-24: the write can fail (permissions, a full disk, a
    // vanished parent dir — `save_notify_credential` already logs the path,
    // never the secret, at `warn` on error). Previously this result was
    // discarded and the redirect always claimed success, so a user whose
    // token never reached disk was told it had and then found notifications
    // silently not working. The switches and destination below still save
    // independently of this outcome; only the redirect distinguishes it.
    let mut notify_credential_save_failed = false;
    if let Some(secret) = form
        .notify_telegram_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if mdview_core::config::save_notify_credential(&cred_path, secret).is_err() {
            notify_credential_save_failed = true;
        }
    }
    let _ = cfg.save_to(&config_path);

    // The switches control live tasks, not just stored values (D7): every
    // save reconciles `terminal_background` against the just-written config,
    // so turning a switch on or off takes effect immediately, with no
    // restart, and turning one off stops exactly what it started.
    let telegram = telegram_credentials(&cfg.terminal, st.config_data_dir.as_deref());
    st.terminal_background.reconcile(
        &cfg.terminal,
        st.herdr.clone(),
        st.notify_store.clone(),
        telegram,
    );

    // agent-terminal-24: a failed credential write redirects to the
    // failure flag instead of `saved=1` — never both, so the settings page
    // never shows the success banner for a save that didn't happen. Neither
    // query value ever carries the credential itself (`SavedFlag`'s doc
    // comment).
    if notify_credential_save_failed {
        Redirect::to("/settings?notify_error=1").into_response()
    } else {
        Redirect::to("/settings?saved=1").into_response()
    }
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
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };

    // D3: the bee entry point (the Bee board card) appears only when the
    // project's root contains a `.bee/` directory — a plain presence check,
    // not a full store read (the page it links to, `/p/:id/_bee`, does the
    // actual reading).
    //
    // D6/agent-terminal-8: `project_home_page` is the only page carrying the
    // Terminal tab strip, so it now renders for EVERY registered project,
    // not only bee ones — a non-bee project used to redirect straight to its
    // entry file (which has no tab strip at all), making the terminal
    // invisible to anyone whose project is an ordinary docs folder. A
    // non-bee project with zero markdown files still falls through to the
    // ordinary not-found page, unchanged from before; a bee project renders
    // even with zero files, exactly as it always did.
    let bee = is_bee_project(&project);
    if !bee && files.is_empty() {
        return not_found("project has no markdown files");
    }
    let entry = pick_entry_file(&files).map(|f| f.rel_path.as_str());
    Html(views::project_home_page(&project, entry, bee)).into_response()
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

/// `GET /p/:id/_terminal` (D2/D4/D6) — the gated per-project pane list.
/// `MethodGate<Get>` and `AuthSession` both run before this body ever
/// executes: a wrong method or a missing/stale session never reaches the
/// project lookup, let alone herdr — see the route table's comment on why
/// this is mounted with `any(...)` rather than `.get(...)`. An unknown
/// project id (a valid session, just the wrong id) still gets the ordinary
/// `not_found` page, same as `bee_board` — that truth is about the *route*
/// existing, not about any particular project id being valid.
///
/// A silent or errored herdr socket renders the D6 remedy state — never a
/// raw error, and never an empty pane list that would look identical to a
/// project that genuinely has zero agents running.
///
/// Carried over from agent-terminal-5 (recorded deviation there): the D7
/// `terminal.enabled` switch is documented as what makes panes and screens
/// reachable, but until this cell only the token gate enforced anything —
/// checked here, after the method/session extractors and before the project
/// lookup, so a disabled switch answers exactly like an unrouted path even
/// with a valid session (see `terminal_family_enabled`).
async fn terminal_page(
    _method: terminal_auth::MethodGate<terminal_auth::Get>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    let presets = configured_preset_labels(&st);
    match st.herdr.snapshot().await {
        Ok(snapshot) => {
            // A boundary that fails to construct (e.g. a project registered
            // on top of the hard-deny list) can never accept any pane —
            // fail closed to zero panes, not a crash and not a laxer check.
            let panes = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
                .map(|boundary| project_panes(&snapshot, &boundary))
                .unwrap_or_default();
            Html(views::terminal_page(&project, &panes, &presets)).into_response()
        }
        Err(_) => Html(views::terminal_down_page(&project)).into_response(),
    }
}

/// `GET /p/:id/_transcript` (D2/D4/D6/D9) — the Transcript tab: the same
/// project-scoped, D2 boundary-filtered pane list `terminal_page` builds,
/// rendered with a transcript viewport per pane instead of a screen.
/// `assets/app.js`'s transcript poller fills each one in from
/// `terminal_transcript` below. Guarded and constructed identically to
/// `terminal_page` — same `MethodGate<Get>` + `AuthSession` + D7 switch,
/// same herdr snapshot + D2 boundary, same D6 herdr-down page — because
/// listing *which* panes belong to this project still requires reaching
/// herdr, even though the transcript content itself never does (D9: the
/// transcript is the agent's own on-disk log, read directly, not through
/// herdr). No creation controls here (D8 stays on the Terminal tab only);
/// this tab is read-only.
async fn transcript_page(
    _method: terminal_auth::MethodGate<terminal_auth::Get>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    match st.herdr.snapshot().await {
        Ok(snapshot) => {
            let panes = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
                .map(|boundary| project_panes(&snapshot, &boundary))
                .unwrap_or_default();
            Html(views::transcript_page(&project, &panes)).into_response()
        }
        Err(_) => Html(views::terminal_down_page(&project)).into_response(),
    }
}

/// The configured D8 preset **labels** only — read the same injectable
/// config path every terminal route uses (`terminal_family_enabled`), so a
/// route test never touches the real `~/.mdview`. `terminal_create_agent`
/// reads the full `AgentPreset` list (label + argv) the same way; this is
/// the labels-only view `terminal_page` renders, since the page never needs
/// argv at all.
fn configured_preset_labels(st: &AppState) -> Vec<String> {
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    cfg.terminal
        .agent_presets
        .into_iter()
        .map(|p| p.label)
        .collect()
}

/// Whether the D7 `terminal.enabled` switch is on, read through the same
/// injectable config path every settings route uses (`st.config_data_dir`)
/// so a route test never touches the real `~/.mdview`. Checked on every
/// route in the gated terminal family — `terminal_page` above and
/// `terminal_screen` below — never on `/settings` or
/// `POST /api/terminal-config`/`POST /settings/terminal/token`, which must
/// stay reachable so the switch can be turned back on (per this cell's
/// carried-over instruction).
fn terminal_family_enabled(st: &AppState) -> bool {
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    cfg.terminal.enabled
}

/// The Telegram credentials to build a live notifier from — `Some((token,
/// chat_id))` only when both the destination (`cfg.notify_chat_id`, an
/// ordinary `Config` field) and the credential (its own owner-only file
/// beside the config — P1's rule, extended to this second secret) are
/// present and non-empty. `None` in every other case, which is exactly the
/// condition under which `notify::TelegramNotifier::new` falls back to the
/// null channel inside `TerminalBackground::reconcile` — so a configuration
/// missing either half never attempts a delivery, whatever the switch says.
fn telegram_credentials(
    cfg: &mdview_core::config::TerminalConfig,
    config_data_dir: Option<&std::path::Path>,
) -> Option<(String, String)> {
    let cred_path = mdview_core::config::notify_credential_path_override(config_data_dir);
    let token = mdview_core::config::load_notify_credential(&cred_path)?;
    let chat_id = cfg.notify_chat_id.clone()?;
    if token.trim().is_empty() || chat_id.trim().is_empty() {
        return None;
    }
    Some((token, chat_id))
}

/// The exact wording `terminal_down_page` renders for D6 — shared so the
/// screen endpoint's herdr-down answer and the page's own down state read
/// identically to whatever surfaces them.
const HERDR_DOWN_REMEDY: &str = "herdr is not running";

/// `GET /p/:id/_terminal/:pane_id/screen` (D2/D3/D4/D6) — one pane's current
/// screen, polled by the client in `assets/app.js`. Modeled on herdr-go's
/// `ScreenBody { text, revision }` (`herdr-go/src/web/screen.rs`), but the
/// `text` field now carries safe, escaped HTML rather than raw text
/// (agent-terminal-12): `mdview_core::ansi::to_html` translates herdr's raw
/// ANSI screen into `<span class="ansi-…">` markup server-side — text is
/// HTML-escaped before any markup wraps it, and any escape sequence the
/// translator does not model (cursor movement, OSC titles, …) is dropped
/// rather than ever reaching the page. `revision` is unchanged; the client
/// still compares it to skip a redundant repaint.
///
/// Guarded exactly like `terminal_page`: `MethodGate<Get>` + `AuthSession`
/// run before this body, then the D7 enabled switch, then the same D2
/// containment boundary `terminal_page` uses — a pane id is only ever read
/// if it is already present in this project's own boundary-filtered pane
/// list, never trusted from the URL alone. A pane that existed when the
/// page listed it but is gone by the time this fires (or was never in this
/// project) gets the ordinary not-found page, distinct from herdr itself
/// being unreachable.
async fn terminal_screen(
    _method: terminal_auth::MethodGate<terminal_auth::Get>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    let snapshot = match st.herdr.snapshot().await {
        Ok(s) => s,
        Err(_) => return herdr_down_response(),
    };
    let in_project = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
        .map(|boundary| project_panes(&snapshot, &boundary))
        .unwrap_or_default()
        .iter()
        .any(|p| p.pane_id == pane_id);
    if !in_project {
        return not_found("pane not found");
    }
    match st.herdr.read_pane(&pane_id, herdr::ReadSource::Visible, 0).await {
        Ok(read) => {
            Json(json!({ "text": mdview_core::ansi::to_html(&read.text), "revision": read.revision }))
                .into_response()
        }
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        // Any other herdr failure (socket gone, protocol mismatch, a
        // request-level error) collapses to the same D6 remedy `terminal_page`
        // renders for a silent socket — the client shows it verbatim rather
        // than a blank screen, and never a raw error type.
        Err(_) => herdr_down_response(),
    }
}

/// The JSON answer `terminal_screen` gives a poller while herdr is
/// unreachable — a `502` (not `200` with empty text, which would be
/// indistinguishable from a genuinely blank screen) carrying the same
/// wording `terminal_down_page` renders.
fn herdr_down_response() -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "error": HERDR_DOWN_REMEDY })),
    )
        .into_response()
}

/// Shared by every write route below (`terminal_input`, `terminal_keys`) and
/// mirrors the read side's own check in `terminal_screen`: a pane id is only
/// ever acted on if it is already present in this project's own D2
/// containment-boundary-filtered pane list, never trusted from the URL
/// alone. Returns the project on success; a `Response` (project-not-found,
/// herdr-down, or pane-not-found) on any refusal, so callers `return` it
/// unchanged via `?` in spirit — used as `match ... { Ok(p) => p, Err(r) =>
/// return r }` at each call site.
async fn project_and_verify_pane_in_boundary(
    st: &AppState,
    id: &str,
    pane_id: &str,
) -> std::result::Result<mdview_core::domain::Project, Response> {
    let Ok(Some(project)) = st.engine.get_project(id) else {
        return Err(not_found("project not found"));
    };
    let snapshot = match st.herdr.snapshot().await {
        Ok(s) => s,
        Err(_) => return Err(herdr_down_response()),
    };
    let in_project = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
        .map(|boundary| project_panes(&snapshot, &boundary))
        .unwrap_or_default()
        .iter()
        .any(|p| p.pane_id == pane_id);
    if !in_project {
        return Err(not_found("pane not found"));
    }
    Ok(project)
}

/// `project_and_verify_pane_in_boundary`'s sibling for routes that need the
/// pane's own cwd *value*, not just a membership check — the transcript
/// reader is keyed on cwd, not pane id (agent-terminal-16). Same D2
/// containment boundary, same refusal shapes (project-not-found,
/// herdr-down, pane-not-found): a pane id is only ever resolved to a cwd if
/// it is already present in this project's own boundary-filtered pane list,
/// never trusted from the URL alone.
async fn project_pane_cwd_in_boundary(
    st: &AppState,
    id: &str,
    pane_id: &str,
) -> std::result::Result<String, Response> {
    let Ok(Some(project)) = st.engine.get_project(id) else {
        return Err(not_found("project not found"));
    };
    let snapshot = match st.herdr.snapshot().await {
        Ok(s) => s,
        Err(_) => return Err(herdr_down_response()),
    };
    let panes = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
        .map(|boundary| project_panes(&snapshot, &boundary))
        .unwrap_or_default();
    panes
        .iter()
        .find(|p| p.pane_id == pane_id)
        .map(|p| p.cwd.clone())
        .ok_or_else(|| not_found("pane not found"))
}

#[derive(serde::Deserialize, Default)]
struct TranscriptQuery {
    /// The opaque cursor `terminal_transcript` last handed back, held
    /// client-side only (nothing about the transcript is persisted
    /// server-side). Absent on the first poll for a pane, which backfills
    /// the tail the same way `mdview_core::transcript::read_activity`'s
    /// `None` case does.
    cursor: Option<String>,
}

/// `GET /p/:id/_terminal/:pane_id/transcript?cursor=...` (D2/D4/D6/D9) — the
/// gap-free activity channel beside `terminal_screen`'s polled screen.
/// Guarded identically: `MethodGate<Get>` + `AuthSession` run before this
/// body, then the D7 enabled switch, then the same D2 containment boundary
/// as every other pane-scoped route — via `project_pane_cwd_in_boundary`
/// rather than `project_and_verify_pane_in_boundary`, since this route needs
/// the pane's resolved cwd itself (this cell's own truth: a session viewing
/// project A must never read an agent's transcript in project B).
///
/// `mdview_core::transcript`'s `parse_cursor` (agent-terminal-15) carries
/// its own guard against a cursor escaping the per-cwd project directory —
/// that is a second line, not a substitute for the D2 check above.
///
/// Nothing is persisted server-side: `chunk.cursor` is the client's to hold
/// and hand back (`assets/app.js`'s transcript poller). `st.transcript_root`
/// only overrides the *default* Claude Code projects root, for a
/// route-level test — the same seam `st.config_data_dir` gives the settings
/// routes over `~/.mdview`; `None` in production resolves exactly where
/// Claude Code itself writes.
///
/// Each returned line is routed through `mdview_core::ansi::to_html`, the
/// same translator `terminal_screen` uses. `transcript.rs`'s own `clip()`
/// already strips any raw ANSI from a rendered record before it ever
/// reaches this handler, so today this call only HTML-escapes — but it
/// keeps every string this route (and the client's `innerHTML` assignment
/// in `assets/app.js`) ever emits on one server-side escaping path, the way
/// the screen route already does, rather than trusting a record to already
/// be safe. A record is never emitted raw.
///
/// D6: a pane with no transcript file yet (a fresh agent that has written
/// nothing) answers `200` with `available: false` — a named, successful
/// state, never the herdr-down error shape `terminal_screen` uses for an
/// unreachable socket, and never an indistinguishable `lines: []`, which
/// would read the same as "caught up, nothing new since the last poll".
async fn terminal_transcript(
    _method: terminal_auth::MethodGate<terminal_auth::Get>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
    Query(q): Query<TranscriptQuery>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    let cwd = match project_pane_cwd_in_boundary(&st, &id, &pane_id).await {
        Ok(cwd) => cwd,
        Err(refusal) => return refusal,
    };
    let result = match st.transcript_root.as_deref() {
        Some(root) => mdview_core::transcript::read_activity_at(root, &cwd, q.cursor.as_deref()),
        None => mdview_core::transcript::read_activity(&cwd, q.cursor.as_deref()),
    };
    match result {
        Ok(chunk) => {
            let lines: Vec<String> = chunk
                .lines
                .iter()
                .map(|l| mdview_core::ansi::to_html(l))
                .collect();
            Json(json!({ "available": true, "lines": lines, "cursor": chunk.cursor })).into_response()
        }
        Err(mdview_core::transcript::TranscriptError::NotAvailable) => {
            Json(json!({ "available": false })).into_response()
        }
        Err(mdview_core::transcript::TranscriptError::BadCursor) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "bad activity cursor" })),
        )
            .into_response(),
        Err(mdview_core::transcript::TranscriptError::Io(_)) => transcript_read_failed_response(),
    }
}

/// The JSON answer `terminal_transcript` gives when the transcript file
/// itself fails to read — a genuine IO error, never the same thing as
/// `TranscriptError::NotAvailable` (D6's ordinary "no transcript yet" state,
/// answered `200`, not this). The raw `std::io::Error` — which can carry a
/// filesystem path fragment — is dropped rather than ever reaching the page.
fn transcript_read_failed_response() -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "error": "transcript read failed" })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct ReplyBody {
    text: String,
    /// Whether to press Enter after the text (a separate herdr call — see
    /// `Herdr::send_input`'s send≠submit doc). Deliberately defaulting to
    /// `false` (`#[serde(default)]` on a `bool`) — the opposite of
    /// herdr-go's `ReplyBody`, which defaults `submit` to `true` — so an
    /// omitted flag never accidentally submits and text can be staged in a
    /// pane without being sent, exactly as this cell's action requires.
    #[serde(default)]
    submit: bool,
}

/// `POST /p/:id/_terminal/:pane_id/input` (D3/D4) — a free-text reply into
/// a pane. Modeled on herdr-go's `ReplyBody { text, submit }`
/// (`herdr-go/src/web/screen.rs`): staging text into the pane's composer and
/// pressing Enter are two separate herdr calls (`Herdr::send_input` makes
/// both, only the second when `submit` is set), so `submit` absent leaves
/// the text staged without ever being sent.
///
/// Guarded exactly like `terminal_screen`: `MethodGate<Post>` + `AuthSession`
/// run before this body, then the D7 enabled switch, then the same D2
/// containment boundary via `project_and_verify_pane_in_boundary`.
async fn terminal_input(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
    Json(body): Json<ReplyBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    if let Err(refusal) = project_and_verify_pane_in_boundary(&st, &id, &pane_id).await {
        return refusal;
    }
    match st.herdr.send_input(&pane_id, &body.text, body.submit).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        // Any other herdr failure collapses to the same D6 remedy the
        // screen poll uses — never a raw error type.
        Err(_) => herdr_down_response(),
    }
}

#[derive(serde::Deserialize)]
struct KeysBody {
    /// herdr key names to press in order (e.g. `["down", "enter"]`) — driving
    /// a TUI option menu the free-text reply can't reach.
    keys: Vec<String>,
}

/// agent-terminal-11: the most named keys a single `/keys` request may carry.
/// `body.keys` was unbounded — herdr forwards each name as its own action, so
/// nothing capped how much work (or pane-visible input) one HTTP request
/// could trigger. 1000 matches herdr's own server-side cap on `pane.read`'s
/// `lines` (`SocketHerdr::read_pane`), the one other place this codebase
/// bounds a herdr-bound list.
const MAX_KEYS_PER_REQUEST: usize = 1000;

/// The refusal `terminal_keys`/`unassigned_terminal_keys` give a request
/// whose `keys` list exceeds `MAX_KEYS_PER_REQUEST` — a `400`, not the
/// terminal family's opaque-404 (this is a validation failure on an already
/// gated, already-authenticated request, not an auth refusal).
fn keys_too_long_response(len: usize) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": format!("too many keys in one request ({len} > {MAX_KEYS_PER_REQUEST})")
        })),
    )
        .into_response()
}

/// `POST /p/:id/_terminal/:pane_id/keys` (D3/D4) — named key presses into a
/// pane, for menu navigation the free-text reply above can't reach (arrow
/// keys, Enter, Escape, Tab, …). Modeled on herdr-go's `KeysBody { keys }`.
/// Guarded identically to `terminal_input`.
async fn terminal_keys(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
    Json(body): Json<KeysBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    if body.keys.len() > MAX_KEYS_PER_REQUEST {
        return keys_too_long_response(body.keys.len());
    }
    if let Err(refusal) = project_and_verify_pane_in_boundary(&st, &id, &pane_id).await {
        return refusal;
    }
    match st.herdr.send_keys(&pane_id, &body.keys).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        Err(_) => herdr_down_response(),
    }
}

#[derive(serde::Deserialize)]
struct CreatePaneBody {}

/// `POST /p/:id/_terminal/create/agent` body. The agent is named by an
/// operator-configured preset **label**, never by `argv` directly (D8/P4):
/// the argv is operator-authored config (`mdview_core::config::AgentPreset`)
/// the label keys into, and the request cannot influence it — no
/// `argv`/`env`/`cwd` field is declared here at all, so serde has nowhere to
/// put one even if a client sends it.
#[derive(serde::Deserialize)]
struct CreateAgentBody {
    preset: String,
}

/// Resolve the herdr workspace `terminal_create_pane`/`terminal_create_agent`
/// target: the first workspace in the snapshot whose D2 anchor
/// (`Snapshot::anchor_cwd_for_workspace`) validates against this project's
/// own containment boundary — the same `Boundary` `project_panes` uses
/// above, just applied to a workspace's own anchor rather than an
/// individual pane's cwd. `None` when no such workspace exists: callers
/// refuse with 409 rather than ever falling back to another directory (this
/// cell's action) — in particular, `Herdr::agent_start`'s own documented
/// `cwd: None` fallback (herdr's own process directory) is never reached,
/// because neither caller below ever invokes `tab_create`/`agent_start`
/// without first resolving a concrete `cwd` here.
fn project_creation_destination(
    snapshot: &herdr::Snapshot,
    boundary: &mdview_core::paths_boundary::Boundary,
) -> Option<(String, String)> {
    snapshot.workspaces.iter().find_map(|w| {
        let anchor = snapshot.anchor_cwd_for_workspace(&w.workspace_id)?;
        let resolved = boundary
            .validate_existing(std::path::Path::new(&anchor))
            .ok()?;
        Some((w.workspace_id.clone(), resolved.to_string_lossy().into_owned()))
    })
}

/// The `409` `terminal_create_pane`/`terminal_create_agent` give when this
/// project has no herdr workspace whose anchor resolves under its own root
/// — never a silent fallback to any other directory (this cell's action).
fn destination_unresolved_response(project_id: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": format!(
                "project {project_id} has no herdr workspace with a resolved working \
                 directory under its own root; refusing to start a process in an \
                 arbitrary directory"
            ),
        })),
    )
        .into_response()
}

/// The `400` `terminal_create_agent` gives a preset label the operator never
/// configured — checked before herdr is ever called, mirroring herdr-go's
/// own rule (`herdr-go/src/web/create.rs`).
fn unknown_preset_response(label: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": format!("unknown agent preset: {label}") })),
    )
        .into_response()
}

/// Map a herdr port error onto the create routes' HTTP surface, mirroring
/// herdr-go's own `herdr_error_response` (`herdr-go/src/web/create.rs`)
/// exactly: a destination that no longer exists by the time herdr is
/// actually called is `409` — never `404`, which stays reserved for the
/// opaque unauthenticated answer `terminal_auth::opaque_404` gives — and
/// everything else collapses to `502` carrying the message, the same
/// "named remedy, never a raw error" rule `herdr_down_response` already
/// applies elsewhere in this file.
fn create_error_response(err: herdr::HerdrError) -> Response {
    let conflict = matches!(err, herdr::HerdrError::WorkspaceNotFound { .. })
        || matches!(
            &err,
            herdr::HerdrError::Remote { code, .. }
                if code == "agent_placement_not_found" || code == "agent_placement_conflict"
        );
    let status = if conflict {
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_GATEWAY
    };
    (status, Json(json!({ "error": err.to_string() }))).into_response()
}

/// `POST /p/:id/_terminal/create/pane` (D8/P4) — open a plain shell in this
/// project. Modeled on herdr-go's `POST /api/panes`
/// (`herdr-go/src/web/create.rs`), adapted to mdview's project-scoped model
/// (D2 — "the project is the frame"): herdr-go's request body names the
/// destination `workspace_id` itself; here the URL's project id is the only
/// destination input, and the workspace is resolved server-side by
/// `project_creation_destination` against this project's own D2 containment
/// boundary, so a session can never aim a creation at a workspace outside
/// the project it is looking at. The body is deliberately empty: a shell
/// takes no command, and no `cwd`/`argv`/`env` field is declared to receive
/// anything a client might try to send.
///
/// Guarded exactly like every other gated terminal route: `MethodGate<Post>`
/// + `AuthSession` run before this body, then the D7 enabled switch, then
/// the project lookup.
async fn terminal_create_pane(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(_body): Json<CreatePaneBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    let snapshot = match st.herdr.snapshot().await {
        Ok(s) => s,
        Err(_) => return herdr_down_response(),
    };
    let Ok(boundary) = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
    else {
        return destination_unresolved_response(&project.id);
    };
    let Some((workspace_id, cwd)) = project_creation_destination(&snapshot, &boundary) else {
        return destination_unresolved_response(&project.id);
    };
    match st.herdr.tab_create(&workspace_id, Some(&cwd)).await {
        Ok(created) => (
            StatusCode::OK,
            Json(json!({ "tab_id": created.tab_id, "pane_id": created.pane_id })),
        )
            .into_response(),
        Err(e) => create_error_response(e),
    }
}

/// `POST /p/:id/_terminal/create/agent` (D8/P4) — start an agent in this
/// project, named by an operator-configured preset **label**, never by
/// `argv` directly: the argv is operator-authored config
/// (`mdview_core::config::AgentPreset`) the label keys into, and no
/// `argv`/`env`/`cwd` field is deserialized from the request at all — there
/// is no field present to receive any of the three (see `CreateAgentBody`).
/// An unknown label is refused with `400` before herdr is ever called.
/// Destination resolution mirrors `terminal_create_pane` exactly (same
/// `project_creation_destination` call, same D2 containment boundary) — and,
/// critically, `cwd` is always passed as `Some(resolved)`, never `None`:
/// `Herdr::agent_start`'s own documented `None` fallback is herdr's own
/// process directory, an arbitrary folder this route must never let a
/// request reach.
async fn terminal_create_agent(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateAgentBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    let Some(preset) = cfg.terminal.agent_presets.iter().find(|p| p.label == body.preset) else {
        return unknown_preset_response(&body.preset);
    };
    let argv = preset.argv.clone();
    let snapshot = match st.herdr.snapshot().await {
        Ok(s) => s,
        Err(_) => return herdr_down_response(),
    };
    let Ok(boundary) = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
    else {
        return destination_unresolved_response(&project.id);
    };
    let Some((workspace_id, cwd)) = project_creation_destination(&snapshot, &boundary) else {
        return destination_unresolved_response(&project.id);
    };
    match st.herdr.agent_start(&workspace_id, Some(&cwd), &argv).await {
        Ok(started) => (
            StatusCode::OK,
            Json(json!({
                "tab_id": started.tab_id,
                "pane_id": started.pane_id,
                "name": started.name,
            })),
        )
            .into_response(),
        Err(e) => create_error_response(e),
    }
}

/// Join each of the snapshot's agents to its own pane's working directory —
/// `Agent` carries none directly (see `herdr::wire::Pane`'s doc: the folder
/// lives on the *pane*, joined by `pane_id`) — and keep only the ones the D2
/// containment boundary accepts under this project's root. The boundary does
/// the actual decision (symlink resolution, component-wise containment,
/// fail-closed on any ambiguity); this function only performs the join and
/// discards anything the boundary refuses or that has no resolvable pane at
/// all. `foreground_cwd` is not consulted here — `cwd` is the pane's own
/// working directory, the literal quantity D2 names, and every panel this
/// cell builds/tests sets it explicitly.
fn project_panes(
    snapshot: &herdr::Snapshot,
    boundary: &mdview_core::paths_boundary::Boundary,
) -> Vec<views::TerminalPaneView> {
    snapshot
        .agents
        .iter()
        .filter_map(|agent| {
            let pane = snapshot.panes.iter().find(|p| p.pane_id == agent.pane_id)?;
            let raw_cwd = pane.cwd.as_deref()?;
            let resolved = boundary
                .validate_existing(std::path::Path::new(raw_cwd))
                .ok()?;
            Some(views::TerminalPaneView {
                pane_id: agent.pane_id.clone(),
                kind: agent.kind.clone(),
                name: agent.name.clone(),
                status: agent.status.as_str().to_string(),
                title: agent.title.clone(),
                cwd: resolved.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

/// D5's partition: every herdr pane whose working directory sits under **no**
/// registered project's D2 containment boundary. Computed as the complement
/// of the union of each project's own `project_panes` result — the same
/// boundary check `terminal_page` runs per project — rather than building
/// one combined `Boundary` over every registered root at once. That matters:
/// `Boundary::new` fails closed on an empty or invalid root set, so if any
/// single registered project's root were unconstructible (e.g. sitting on
/// the hard-deny list) a combined boundary would fail to construct entirely,
/// and every pane — including ones that plainly belong to a *different*,
/// perfectly valid project — would wrongly render here. Per-project
/// computation keeps one broken project's boundary from ever leaking another
/// project's panes into this group, or hiding them from their own project's
/// page.
///
/// agent-terminal-11: a project whose own boundary fails to construct used
/// to contribute nothing to `assigned`, which meant *that project's own*
/// panes fell through and rendered here as if they belonged to no project —
/// the exact widening this group's session gate is the last line of defense
/// against (per P6, there is no second containment check for this group; the
/// gate is what authorizes). There is no way to tell, without a working
/// boundary, which of the project's real panes those were — so the whole
/// group fails closed to empty rather than guess, the same "fail closed to
/// zero, not a crash and not a laxer check" rule `terminal_page` already
/// applies to a single unconstructible project.
///
/// A pane's raw, unvalidated cwd is used for display here (or left empty if
/// herdr never reported one) — never resolved through any `Boundary`, since
/// per P6 there is no containment claim to make for a pane that belongs to
/// no project; the boundary check above is only ever used to decide
/// membership, never to canonicalize an unassigned pane's path.
fn unassigned_panes(
    snapshot: &herdr::Snapshot,
    projects: &[mdview_core::domain::Project],
) -> Vec<views::TerminalPaneView> {
    let mut assigned: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in projects {
        match mdview_core::paths_boundary::Boundary::new(vec![p.root_path.clone()]) {
            Ok(boundary) => {
                assigned.extend(project_panes(snapshot, &boundary).into_iter().map(|pane| pane.pane_id));
            }
            Err(_) => {
                // Fail closed: this project's own panes cannot be told apart
                // from a genuinely unassigned one without a working
                // boundary, so the whole group renders empty rather than
                // risk leaking them in.
                return Vec::new();
            }
        }
    }

    snapshot
        .agents
        .iter()
        .filter(|agent| !assigned.contains(&agent.pane_id))
        .map(|agent| {
            let cwd = snapshot
                .panes
                .iter()
                .find(|p| p.pane_id == agent.pane_id)
                .and_then(|p| p.cwd.clone())
                .unwrap_or_default();
            views::TerminalPaneView {
                pane_id: agent.pane_id.clone(),
                kind: agent.kind.clone(),
                name: agent.name.clone(),
                status: agent.status.as_str().to_string(),
                title: agent.title.clone(),
                cwd,
            }
        })
        .collect()
}

/// `GET /_terminal/unassigned` (D5/D4/D6) — the gated cross-project pane
/// list. Guarded identically to `terminal_page`: `MethodGate<Get>` +
/// `AuthSession` run before this body (see the route table's comment on
/// `any(...)` vs `.get(...)`), then the D7 enabled switch — every registered
/// project's own boundary check happens inside `unassigned_panes`, not here.
/// A silent herdr socket renders the same D6 remedy `terminal_page` uses.
///
/// agent-terminal-11: a registry read failure used to fall through
/// `unwrap_or_default()` to an empty project list, which made every pane in
/// the whole snapshot — including ones that plainly belong to a registered
/// project — read as unassigned. Fail closed instead: an unreadable
/// registry renders the group empty, the same as `unassigned_panes` failing
/// closed on an unconstructable project boundary.
async fn unassigned_terminal_page(
    _method: terminal_auth::MethodGate<terminal_auth::Get>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    let Ok(projects) = st.engine.list_projects() else {
        return Html(views::unassigned_terminal_page(&[])).into_response();
    };
    match st.herdr.snapshot().await {
        Ok(snapshot) => {
            let panes = unassigned_panes(&snapshot, &projects);
            Html(views::unassigned_terminal_page(&panes)).into_response()
        }
        Err(_) => Html(views::unassigned_terminal_down_page()).into_response(),
    }
}

/// Shared by `unassigned_terminal_screen`, `unassigned_terminal_input` and
/// `unassigned_terminal_keys` — mirrors `project_and_verify_pane_in_boundary`
/// but for the Unassigned group: a pane id is only ever read or acted on if
/// it is already present in this request's own freshly computed
/// `unassigned_panes` result, never trusted from the URL alone. A pane that
/// is actually inside some registered project's boundary refuses here with
/// the ordinary not-found — the two groups partition, they never overlap.
///
/// agent-terminal-11: same fail-closed rule as `unassigned_terminal_page` —
/// a registry read failure refuses the pane (ordinary not-found) rather than
/// falling through to an empty project list, which would have made every
/// pane in the system, including ones inside a real project's boundary,
/// verify as unassigned.
async fn verify_pane_is_unassigned(st: &AppState, pane_id: &str) -> std::result::Result<(), Response> {
    let Ok(projects) = st.engine.list_projects() else {
        return Err(not_found("pane not found"));
    };
    let snapshot = match st.herdr.snapshot().await {
        Ok(s) => s,
        Err(_) => return Err(herdr_down_response()),
    };
    let in_unassigned = unassigned_panes(&snapshot, &projects)
        .iter()
        .any(|p| p.pane_id == pane_id);
    if !in_unassigned {
        return Err(not_found("pane not found"));
    }
    Ok(())
}

/// `GET /_terminal/unassigned/:pane_id/screen` (D5/D4/D6) — one unassigned
/// pane's current screen, the same shape `terminal_screen` returns for a
/// project's own pane. Guarded identically: `MethodGate<Get>` +
/// `AuthSession`, then the D7 switch, then `verify_pane_is_unassigned`.
async fn unassigned_terminal_screen(
    _method: terminal_auth::MethodGate<terminal_auth::Get>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path(pane_id): Path<String>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    if let Err(refusal) = verify_pane_is_unassigned(&st, &pane_id).await {
        return refusal;
    }
    match st.herdr.read_pane(&pane_id, herdr::ReadSource::Visible, 0).await {
        Ok(read) => {
            Json(json!({ "text": mdview_core::ansi::to_html(&read.text), "revision": read.revision }))
                .into_response()
        }
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        Err(_) => herdr_down_response(),
    }
}

/// `POST /_terminal/unassigned/:pane_id/input` (D3/D5/D4) — the Unassigned
/// group's write path from agent-terminal-9: free-text reply, same
/// `ReplyBody { text, submit }` shape and the same send≠submit semantics as
/// `terminal_input`. Guarded identically, via `verify_pane_is_unassigned`.
async fn unassigned_terminal_input(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path(pane_id): Path<String>,
    Json(body): Json<ReplyBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    if let Err(refusal) = verify_pane_is_unassigned(&st, &pane_id).await {
        return refusal;
    }
    match st.herdr.send_input(&pane_id, &body.text, body.submit).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        Err(_) => herdr_down_response(),
    }
}

/// `POST /_terminal/unassigned/:pane_id/keys` (D3/D5/D4) — the Unassigned
/// group's other write path from agent-terminal-9: named key presses, same
/// `KeysBody { keys }` shape as `terminal_keys`. Guarded identically, via
/// `verify_pane_is_unassigned`.
async fn unassigned_terminal_keys(
    _method: terminal_auth::MethodGate<terminal_auth::Post>,
    _session: terminal_auth::AuthSession,
    State(st): State<AppState>,
    Path(pane_id): Path<String>,
    Json(body): Json<KeysBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_auth::opaque_404();
    }
    if body.keys.len() > MAX_KEYS_PER_REQUEST {
        return keys_too_long_response(body.keys.len());
    }
    if let Err(refusal) = verify_pane_is_unassigned(&st, &pane_id).await {
        return refusal;
    }
    match st.herdr.send_keys(&pane_id, &body.keys).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        Err(_) => herdr_down_response(),
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
    use std::time::Duration;
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
            // A fresh FakeHerdr per state — no route test ever reaches a
            // real herdr socket. Tests that need specific panes replace this
            // with their own `FakeHerdr` (see the `terminal_route_*` tests
            // below in this module).
            herdr: Arc::new(crate::herdr::fake::FakeHerdr::new()),
            // No route test reads the real `~/.claude/projects` by default —
            // transcript tests set this explicitly (see
            // `transcript_root_dir` below).
            transcript_root: None,
            // A fresh manager per state, mirroring `terminal_auth` above —
            // no route test shares live background tasks across states.
            terminal_background: Arc::new(crate::TerminalBackground::new()),
            // In-memory outbox — no route test ever touches a real sqlite
            // file for this.
            notify_store: Arc::new(
                mdview_core::notify_store::NotifyStore::open_in_memory().unwrap(),
            ),
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

    /// One entry in a `snapshot_tree` — either a directory or a file with
    /// its content. Recording directories closes a hole in the D4
    /// read-only probe: a reader that calls `create_dir_all` before
    /// `read_dir` (the obvious idiom for listing a directory such as
    /// `.bee/reviews/`) writes into the user's store, but a file-only
    /// snapshot never noticed — the new directory carried no bytes to
    /// diff. `Debug` on this enum keeps `assert_eq!`'s failure message
    /// readable: a lone new directory prints as `Dir("...")`, not as an
    /// unexplained length mismatch.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TreeEntry {
        Dir(String),
        File(String, Vec<u8>),
    }

    impl TreeEntry {
        fn rel_path(&self) -> &str {
            match self {
                TreeEntry::Dir(p) | TreeEntry::File(p, _) => p,
            }
        }
    }

    /// One entry per file (with content) and one entry per directory,
    /// for everything under `dir` — the D4 read-only probe's before/after
    /// snapshot. Sorted by relative path so the comparison is deterministic
    /// regardless of filesystem iteration order.
    fn snapshot_tree(dir: &Path) -> Vec<TreeEntry> {
        fn walk(base: &Path, cur: &Path, out: &mut Vec<TreeEntry>) {
            for entry in std::fs::read_dir(cur).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                if path.is_dir() {
                    out.push(TreeEntry::Dir(rel));
                    walk(base, &path, out);
                } else {
                    let content = std::fs::read(&path).unwrap();
                    out.push(TreeEntry::File(rel, content));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort_by(|a, b| a.rel_path().cmp(b.rel_path()));
        out
    }

    #[test]
    fn snapshot_tree_notices_an_empty_directory_created_under_bee() {
        let root = fresh_root("snapshot-tree-dir");
        write(&root, "README.md", "# hi");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);

        let before = snapshot_tree(&root);
        // No files written — just the directory itself, the shape a
        // `create_dir_all` before a `read_dir` leaves behind.
        std::fs::create_dir_all(root.join(".bee/reviews")).unwrap();
        let after = snapshot_tree(&root);

        assert_ne!(
            before, after,
            "an empty directory created under .bee/ went unnoticed by the read-only probe"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn snapshot_tree_of_an_empty_tree_compares_equal_to_itself() {
        let root = fresh_root("snapshot-tree-empty");
        std::fs::create_dir_all(&root).unwrap();

        assert_eq!(snapshot_tree(&root), snapshot_tree(&root));

        std::fs::remove_dir_all(&root).ok();
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

    // --- bbp-5: the rebuilt top of the board (D5/D6/D7/D8/D9) ---

    /// (happy) The board names the active feature, the lifecycle stepper's
    /// state (a done step, a current step, and a pending step all present),
    /// and the recorded next action.
    #[tokio::test]
    async fn board_names_active_feature_stepper_state_and_next_action() {
        let root = fresh_root("top-happy");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "phase": "shaping",
                "feature": "pm-view-feature",
                "mode": "standard",
                "approved_gates": {"context": true, "shape": false, "execution": false, "review": false},
                "next_action": "Invoke bee-shaping to lock the shape gate."
            }"#,
        );

        let st = build_state();
        let project = register(&st, &root, "top-happy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("pm-view-feature"), "active feature name missing: {body}");
        assert!(
            body.contains("Invoke bee-shaping to lock the shape gate."),
            "next action missing: {body}"
        );
        assert!(
            body.contains("class=\"bee-step bee-step--done\""),
            "expected at least one done step: {body}"
        );
        assert!(
            body.contains("class=\"bee-step bee-step--current\""),
            "expected a current step: {body}"
        );
        assert!(
            body.contains("class=\"bee-step bee-step--pending\""),
            "expected at least one pending step: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) A feature with three of four gates approved renders exactly
    /// three done steps, and the review step — never approved here — reads
    /// as something the human invokes, never as pending automatic work
    /// (D7).
    #[tokio::test]
    async fn board_three_of_four_gates_done_review_rendered_as_user_invoked() {
        let root = fresh_root("top-three-gates");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "feature": "three-gates-feature",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": false}
            }"#,
        );

        let st = build_state();
        let project = register(&st, &root, "top-three-gates");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert_eq!(
            body.matches("class=\"bee-step bee-step--done\"").count(),
            3,
            "expected exactly three done steps: {body}"
        );
        assert!(
            !body.contains("class=\"bee-step bee-step--done\" data-step=\"review\""),
            "review must never render as done here: {body}"
        );
        assert!(
            body.contains("Runs only when you invoke it — never automatic."),
            "the review step must read as user-invoked, never pending automatic work: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (regression, bbp-7 — the live defect) A gate that is currently
    /// approved renders as approved (done), whatever `gate_revoked_at`
    /// records — `gate_revoked_at` is bee's append-style historical anchor
    /// for advisor staleness, not a current-state flag, and a revocation
    /// recorded on an earlier day must never contradict a `true`
    /// `approved_gates` entry recorded after it. This replaces
    /// `board_revoked_execution_gate_does_not_render_as_approved`, which
    /// encoded the opposite (wrong) rule from bbp-5: it asserted that
    /// `approved_gates.execution: true` plus a `gate_revoked_at.execution`
    /// entry rendered Execute as NOT done and carrying "Approved, then
    /// revoked." — exactly this repo's own live board bug.
    #[tokio::test]
    async fn board_currently_approved_gate_renders_approved_despite_earlier_revocation() {
        let root = fresh_root("top-approved-despite-revocation");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "feature": "revoked-then-reapproved-feature",
                "approved_gates": {"context": true, "shape": true, "execution": true, "review": false},
                "gate_revoked_at": {"execution": "2026-08-05T09:51:47.038Z"}
            }"#,
        );

        let st = build_state();
        let project = register(&st, &root, "top-approved-despite-revocation");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("class=\"bee-step bee-step--done\" data-step=\"execution\""),
            "an execution gate that is currently approved must render as done, whatever an earlier gate_revoked_at records: {body}"
        );
        assert!(
            !body.contains("Approved, then revoked."),
            "a currently-approved gate must never carry the revoked wording: {body}"
        );
        assert_eq!(
            body.matches("class=\"bee-step bee-step--done\"").count(),
            3,
            "context, shape and execution are all cleanly approved: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) A gate that is NOT currently approved and carries a
    /// `gate_revoked_at` entry renders as revoked — distinguishable from a
    /// step that was simply never approved (bbp-7).
    #[tokio::test]
    async fn board_unapproved_gate_with_revocation_renders_as_revoked() {
        let root = fresh_root("top-unapproved-revoked-gate");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "feature": "genuinely-revoked-feature",
                "approved_gates": {"context": true, "shape": true, "execution": false, "review": false},
                "gate_revoked_at": {"execution": "2026-08-05T09:51:47.038Z"}
            }"#,
        );

        let st = build_state();
        let project = register(&st, &root, "top-unapproved-revoked-gate");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            !body.contains("class=\"bee-step bee-step--done\" data-step=\"execution\""),
            "an unapproved execution gate must not render as done: {body}"
        );
        assert!(
            body.contains("Approved, then revoked."),
            "an unapproved gate carrying a revocation must read as revoked, not merely unreached: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) A gate that is NOT approved and carries no `gate_revoked_at`
    /// entry renders as not yet reached — never as revoked (bbp-7 honest
    /// empty case).
    #[tokio::test]
    async fn board_unapproved_gate_with_no_revocation_renders_as_not_yet_reached() {
        let root = fresh_root("top-unapproved-never-revoked-gate");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "feature": "never-approved-feature",
                "approved_gates": {"context": true, "shape": true, "execution": false, "review": false}
            }"#,
        );

        let st = build_state();
        let project = register(&st, &root, "top-unapproved-never-revoked-gate");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            !body.contains("class=\"bee-step bee-step--done\" data-step=\"execution\""),
            "an unapproved execution gate must not render as done: {body}"
        );
        assert!(
            body.contains("Not yet approved."),
            "an unapproved gate with no revocation must read as not yet reached: {body}"
        );
        assert!(
            !body.contains("Approved, then revoked."),
            "a gate with no revocation on record must never carry the revoked wording: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A `gate_revoked_at` entry naming a different gate does not
    /// affect this gate's rendering (bbp-7).
    #[tokio::test]
    async fn board_revocation_on_a_different_gate_does_not_affect_this_gate() {
        let root = fresh_root("top-revocation-different-gate");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "feature": "unrelated-revocation-feature",
                "approved_gates": {"context": true, "shape": false, "execution": false, "review": false},
                "gate_revoked_at": {"context": "2026-08-05T09:51:47.038Z"}
            }"#,
        );

        let st = build_state();
        let project = register(&st, &root, "top-revocation-different-gate");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("class=\"bee-step bee-step--done\" data-step=\"context\""),
            "the context gate is currently approved and must render as done despite carrying its own revocation history: {body}"
        );
        assert!(
            !body.contains("class=\"bee-step bee-step--done\" data-step=\"shape\""),
            "the shape gate was never approved: {body}"
        );
        assert!(
            !body.contains("Approved, then revoked."),
            "the shape gate carries no gate_revoked_at entry of its own, so a context-gate revocation must not leak into it: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A feature whose cells are all `dropped` renders an honest
    /// progress state — no division artifact, per D8 (dropped cells count
    /// toward neither the numerator nor the denominator).
    #[tokio::test]
    async fn board_feature_with_all_dropped_cells_renders_honest_progress_no_division_artifact() {
        let root = fresh_root("top-all-dropped");
        write(&root, ".bee/state.json", r#"{"feature": "all-dropped-feature"}"#);
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json("d1", "all-dropped-feature", "dropped", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "top-all-dropped");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("all-dropped-feature"), "{body}");
        assert!(
            body.contains("No live cells recorded for this feature yet."),
            "expected an honest progress state: {body}"
        );
        assert!(!body.contains("0/0"), "a division artifact leaked in: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) An empty attention list renders one honest line, never an
    /// empty bordered panel.
    #[tokio::test]
    async fn board_empty_attention_list_renders_one_honest_line() {
        let root = fresh_root("top-attention-empty");
        write(&root, ".bee/cells/a.json", &cell_json("a", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "top-attention-empty");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("Nothing needs attention right now."),
            "expected the honest empty-attention line: {body}"
        );
        assert!(
            !body.contains("bee-cell bee-attention__item"),
            "no attention items should render when the list is empty: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (untrusted) A cell title (rendered inside the attention panel's
    /// "blocked" item, via `compute_attention_items`) and a `next_action`
    /// (rendered inside the "working on now" card) each containing
    /// `< > & "` render as text and never break the surrounding markup.
    #[tokio::test]
    async fn board_untrusted_cell_title_and_next_action_render_as_text() {
        let root = fresh_root("top-untrusted");
        write(
            &root,
            ".bee/state.json",
            r#"{"feature": "untrusted-feature", "next_action": "Fix <script>alert(1)</script> & \"quote\" it."}"#,
        );
        let untrusted_title = "<b>bold</b> & \"quoted\" title";
        let untrusted_cell = format!(
            r#"{{
                "id": "untrusted-cell",
                "feature": "untrusted-feature",
                "lane": "standard",
                "title": {title},
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
                "status": "blocked",
                "tier": "generation",
                "trace": {{"worker": "w1"}}
            }}"#,
            title = serde_json::to_string(untrusted_title).unwrap(),
        );
        write(&root, ".bee/cells/a.json", &untrusted_cell);

        let st = build_state();
        let project = register(&st, &root, "top-untrusted");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("&lt;b&gt;bold&lt;/b&gt; &amp; &quot;quoted&quot; title"),
            "untrusted cell title must render escaped, as text: {body}"
        );
        assert!(!body.contains("<b>bold</b>"), "the raw title tag must not survive: {body}");
        assert!(
            body.contains("Fix &lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quote&quot; it."),
            "untrusted next_action must render escaped, as text: {body}"
        );
        assert!(
            !body.contains("<script>alert(1)</script>"),
            "the raw next_action script tag must not survive: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (security) An absolute path embedded mid-sentence in `route.rationale`
    /// and in `next_action` — the two new free-text fields this cell renders
    /// — must not leak into the board body. The existing leak tests only
    /// cover wholly-path fields; this closes the gap the plan named.
    #[tokio::test]
    async fn board_embedded_absolute_path_in_rationale_and_next_action_does_not_leak() {
        let root = fresh_root("top-security-scrub");
        let secret = root.join("src").join("secret.rs").to_string_lossy().into_owned();
        let secret_escaped = secret.replace('\\', "\\\\");
        let state = format!(
            r#"{{
                "feature": "security-feature",
                "route": {{
                    "class": "feature",
                    "lane": "standard",
                    "flags": [],
                    "product_files": 1,
                    "rationale": "See {rationale_path} before merging.",
                    "updated_at": "2026-08-01T00:00:00.000Z"
                }},
                "next_action": "Read {next_action_path} then continue."
            }}"#,
            rationale_path = secret_escaped,
            next_action_path = secret_escaped,
        );
        write(&root, ".bee/state.json", &state);

        let st = build_state();
        let project = register(&st, &root, "top-security-scrub");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(!body.contains(&secret), "the board leaked an absolute path embedded in free text: {body}");
        assert!(body.contains("src/secret.rs"), "the reduced relative path should still read: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only) The fixture `.bee/` tree is byte-identical after a
    /// request that exercises gates, revocation, route, next_action and a
    /// populated attention panel all at once — the fuller shape the new top
    /// section reads, beyond `reading_never_writes_the_fixtures_bee_tree`'s
    /// minimal fixture.
    #[tokio::test]
    async fn board_top_of_page_reading_is_read_only_with_gates_route_and_attention_populated() {
        let root = fresh_root("top-read-only");
        write(
            &root,
            ".bee/state.json",
            r#"{
                "feature": "read-only-feature",
                "approved_gates": {"context": true, "shape": true, "execution": false, "review": false},
                "route": {
                    "class": "feature",
                    "lane": "standard",
                    "flags": [],
                    "product_files": 2,
                    "rationale": "Small, well-scoped change.",
                    "updated_at": "2026-08-01T00:00:00.000Z"
                },
                "next_action": "Invoke bee-swarming."
            }"#,
        );
        write(
            &root,
            ".bee/cells/blocked.json",
            &timed_cell_json("b1", "read-only-feature", "blocked", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "top-read-only");
        let before = snapshot_tree(&root);

        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let after = snapshot_tree(&root);
        assert_eq!(
            before, after,
            ".bee/ tree changed after a request exercising gates, route, next_action and attention"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only, bbp-8) The fixture `.bee/` tree is byte-identical after a
    /// board request whose fixture CONTAINS a populated `HANDOFF.json` — the
    /// six pre-existing read-only tests each use a fixture with no handoff
    /// file, so they would pass green without the handoff reader ever
    /// running. This one exercises it: the reader must open the file, parse
    /// it and scrub it without ever writing back to it.
    #[tokio::test]
    async fn board_reading_is_read_only_with_a_populated_handoff_present() {
        let root = fresh_root("handoff-read-only");
        write(
            &root,
            ".bee/HANDOFF.json",
            r#"{
                "written_at": "2026-08-06T12:45:21.418Z",
                "next_action": "Resume the next slice of bee-board-pm.",
                "kind": "pause"
            }"#,
        );
        write(
            &root,
            ".bee/cells/blocked.json",
            &timed_cell_json("b1", "handoff-read-only", "blocked", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "handoff-read-only");
        let before = snapshot_tree(&root);

        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_string(resp).await;
        assert!(body.contains("parked"), "the pause handoff should surface as an attention item: body missing expected text");

        let after = snapshot_tree(&root);
        assert_eq!(
            before, after,
            ".bee/ tree changed after a request whose fixture carries a populated HANDOFF.json"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only, bbp-9) The fixture `.bee/` tree is byte-identical after a
    /// board request whose fixture CONTAINS a populated `.bee/config.json` —
    /// the pre-existing read-only tests each use a fixture with no config
    /// file, so they would pass green without the config reader ever
    /// running. This one exercises it: the reader must open the file, parse
    /// it and report the recorded `gate_bypass` level without ever writing
    /// back to it, and without opening `.bee/config.local.json` at all.
    #[tokio::test]
    async fn board_reading_is_read_only_with_a_populated_config_present() {
        let root = fresh_root("config-read-only");
        write(&root, ".bee/config.json", r#"{"gate_bypass": "total"}"#);
        write(
            &root,
            ".bee/cells/c-open.json",
            &timed_cell_json("c1", "config-read-only", "open", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "config-read-only");
        let before = snapshot_tree(&root);

        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_string(resp).await;
        assert!(
            body.contains("recorded") && body.contains("total"),
            "the recorded gate-bypass level should surface as an attention item: body missing expected text"
        );

        let after = snapshot_tree(&root);
        assert_eq!(
            before, after,
            ".bee/ tree changed after a request whose fixture carries a populated config.json"
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

        // The session obtained by logging in with the just-revealed token is
        // real and usable, proving the masking above isn't hiding a second
        // reveal path through it either.
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

        // A real session, obtained by generating the token then logging in
        // with it, succeeds.
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

    /// agent-terminal-18's key_link: the switches control **live tasks**,
    /// not just stored config values. Reconciliation happens synchronously
    /// inside `update_terminal_config`, so this is provable with no sleep or
    /// timing at all — right after each response, `terminal_background`'s
    /// own bookkeeping already reflects the new switch state. With both
    /// switches off (the state before the first POST below) nothing is
    /// running at all — the same D7 guarantee `main.rs`'s
    /// `default_config_starts_nothing` proves at the manager level, proven
    /// here end-to-end through the real gated route.
    #[tokio::test]
    async fn gated_switch_route_starts_and_stops_the_live_background_tasks() {
        let dir = fresh_root("terminal-switches-live-tasks");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());
        let (_token, cookie) = rotate_token(app.clone()).await;

        assert!(!st.terminal_background.supervisor_running());
        assert!(!st.terminal_background.notify_running());

        let on_req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header("content-type", "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie.clone())
            .body(Body::from("enabled=on&supervisor_enabled=on&notify_enabled=on"))
            .unwrap();
        let resp = app.clone().oneshot(on_req).await.unwrap();
        assert!(resp.status().is_redirection());
        assert!(
            st.terminal_background.supervisor_running(),
            "turning the supervisor switch on through the gated route must start the watchdog"
        );
        assert!(
            st.terminal_background.notify_running(),
            "turning the notify switch on through the gated route must start the watcher"
        );

        // Leaving both switch fields off this submission (only `enabled` is
        // carried) must stop both live tasks — no restart needed.
        let off_req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header("content-type", "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie)
            .body(Body::from("enabled=on"))
            .unwrap();
        let resp = app.oneshot(off_req).await.unwrap();
        assert!(resp.status().is_redirection());
        assert!(
            !st.terminal_background.supervisor_running(),
            "turning the supervisor switch off must stop the watchdog without a restart"
        );
        assert!(
            !st.terminal_background.notify_running(),
            "turning the notify switch off must stop the watcher without a restart"
        );

        // agent-terminal-24 (closing agent-terminal-22's named gap): the two
        // `running()` assertions above only read `TerminalBackground`'s own
        // bookkeeping (`slot.lock().unwrap().is_some()`), which
        // `reconcile_*`'s `(false, Some(handle)) => handle.abort()` arm
        // empties via `take()` regardless of whether `abort()` actually does
        // anything — deleting the `abort()` call would still leave them
        // green. `supervisor_ticks`/`notify_ticks` (main.rs, landed by
        // agent-terminal-21) are a real, externally-observable side effect
        // of each loop still running, reachable from here because they are
        // private to the crate root and this module is one of its
        // descendants. This route always reconciles through the fixed
        // production intervals (5s supervisor, 2000ms notify — no test-only
        // fast-interval seam reaches `update_terminal_config`), so proving
        // the tick count stays put after "off" means waiting past both real
        // intervals rather than a short sleep: a short wait would pass
        // whether or not cancellation worked, because production's next
        // tick isn't due yet either way — exactly the "no mutation would
        // make it fail" shape this cell's dispatch warns against.
        //
        // Verified red/green by hand for this cell: temporarily replacing
        // `existing.handle.abort()` with a no-op in both
        // `reconcile_supervisor_with_intervals`'s and
        // `reconcile_notify_with_interval`'s `(false, Some(existing))` arms
        // (main.rs) turned both assertions below red — ticks kept advancing
        // past `_at_off` — while leaving the `running()` assertions above
        // green, and the running-through-a-restart assertions in
        // `main.rs` unaffected. Restored before committing.
        let supervisor_ticks_at_off = st.terminal_background.supervisor_ticks();
        let notify_ticks_at_off = st.terminal_background.notify_ticks();
        tokio::time::sleep(Duration::from_millis(5_300)).await;
        assert_eq!(
            st.terminal_background.supervisor_ticks(),
            supervisor_ticks_at_off,
            "the watchdog kept ticking after the switch was turned off — the bookkeeping \
             slot emptied but the loop itself was not actually cancelled"
        );
        assert_eq!(
            st.terminal_background.notify_ticks(),
            notify_ticks_at_off,
            "the watcher kept ticking after the switch was turned off — the bookkeeping \
             slot emptied but the loop itself was not actually cancelled"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// agent-terminal-22, truth: "With both switches off, starting the
    /// server creates no notification database or sidecar files." Only
    /// `TerminalBackground::reconcile_notify` (the notify switch) ever reads
    /// or writes the outbox — the supervisor switch never touches it — so
    /// `open_notify_store` must not create the real sqlite file (or the WAL
    /// journal `NotifyStore::open`'s WAL mode creates alongside it) until the
    /// config it is given already has `notify_enabled` set. `serve()` itself
    /// binds a real socket and blocks on the watcher, so it is not
    /// unit-testable directly; this drives the same lazy-open decision it
    /// makes, extracted into `open_notify_store` for exactly this reason.
    #[test]
    fn notify_store_opens_lazily_only_once_the_notify_switch_is_on() {
        let dir = fresh_root("notify-store-lazy-open");
        let path = mdview_core::config::notify_store_path_override(Some(&dir));

        let off = mdview_core::config::TerminalConfig::default();
        assert!(!off.notify_enabled, "the switch must default to off");
        let _store = open_notify_store(&off, Some(&dir));
        assert!(
            !path.exists(),
            "opening the store with the notify switch off must not create a database file: {}",
            path.display()
        );

        let on = mdview_core::config::TerminalConfig {
            notify_enabled: true,
            ..Default::default()
        };
        let _store = open_notify_store(&on, Some(&dir));
        assert!(
            path.exists(),
            "opening the store with the notify switch on must create the real outbox: {}",
            path.display()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The destination and credential the settings page's notification
    /// section writes (D7/D9) sit behind the same P3 gate as the switches —
    /// this cell extends `update_terminal_config`'s existing session check
    /// to two more fields on the same endpoint rather than adding a second
    /// one. Mirrors `terminal_switches_require_a_valid_terminal_session`.
    #[tokio::test]
    async fn notify_destination_and_credential_require_a_valid_terminal_session() {
        let dir = fresh_root("terminal-notify-gated");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());
        let cred_path = mdview_core::config::notify_credential_path_override(Some(&dir));

        let notify_req = |cookie: Option<&str>| {
            let mut b = Request::builder()
                .method("POST")
                .uri("/api/terminal-config")
                .header("content-type", "application/x-www-form-urlencoded");
            if let Some(c) = cookie {
                b = b.header(header::COOKIE, c.to_string());
            }
            b.body(Body::from("notify_chat_id=12345&notify_telegram_token=secret-bot-token"))
                .unwrap()
        };

        // No session at all.
        let resp = app.clone().oneshot(notify_req(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // An unknown/stale session cookie.
        let resp = app
            .clone()
            .oneshot(notify_req(Some("mdview_terminal_session=not-a-real-session")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let cfg_after_refusals = Config::load_from(&dir.join("config.toml"));
        assert_eq!(cfg_after_refusals.terminal.notify_chat_id, None);
        assert_eq!(mdview_core::config::load_notify_credential(&cred_path), None);

        // A real session, obtained the same way every other gated-switch
        // test does, succeeds.
        let (_full_token, cookie) = rotate_token(app.clone()).await;
        let resp = app.clone().oneshot(notify_req(Some(&cookie))).await.unwrap();
        assert!(
            resp.status().is_redirection(),
            "a valid terminal session could not save the notification settings, got {}",
            resp.status()
        );

        let saved = Config::load_from(&dir.join("config.toml"));
        assert_eq!(saved.terminal.notify_chat_id.as_deref(), Some("12345"));
        assert_eq!(
            mdview_core::config::load_notify_credential(&cred_path).as_deref(),
            Some("secret-bot-token")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// P3 extended: `POST /api/config` is unauthenticated (D4 leaves it that
    /// way) and its own `SettingsForm` has no `notify_chat_id`/
    /// `notify_telegram_token` field at all — submitting them there must
    /// have zero effect, the same guarantee
    /// `post_api_config_with_terminal_fields_leaves_every_switch_unchanged`
    /// proves for the switches.
    #[tokio::test]
    async fn post_api_config_with_notify_fields_leaves_destination_and_credential_unchanged() {
        let dir = fresh_root("terminal-notify-not-via-api-config");
        let st = build_state_with_dir(&dir);

        let req = Request::builder()
            .method("POST")
            .uri("/api/config")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(
                "port=58313&notify_chat_id=12345&notify_telegram_token=secret-bot-token",
            ))
            .unwrap();
        let resp = router(st).oneshot(req).await.unwrap();
        assert!(resp.status().is_redirection());

        let saved = Config::load_from(&dir.join("config.toml"));
        assert_eq!(saved.server.port, 58313, "the legitimate field was not saved");
        assert_eq!(
            saved.terminal.notify_chat_id, None,
            "POST /api/config set the notification destination"
        );
        let cred_path = mdview_core::config::notify_credential_path_override(Some(&dir));
        assert_eq!(
            mdview_core::config::load_notify_credential(&cred_path),
            None,
            "POST /api/config wrote a Telegram credential file"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The credential must never appear on `GET /api/config`, before or
    /// after it is saved — mirrors
    /// `api_config_never_contains_the_token_value_before_or_after_generation`,
    /// against the real submitted secret value rather than a hardcoded
    /// guess (a leak assertion must be written against the value that would
    /// leak — `docs/history/learnings/20260805-toothless-security-assertions.md`).
    #[tokio::test]
    async fn api_config_never_contains_the_notify_credential() {
        let dir = fresh_root("terminal-notify-cred-not-in-api-config");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());
        let (_token, cookie) = rotate_token(app.clone()).await;

        let secret = "unique-telegram-secret-98765";
        let req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header("content-type", "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie)
            .body(Body::from(format!(
                "notify_chat_id=555&notify_telegram_token={secret}"
            )))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_redirection());

        let resp = get(app, "/api/config").await;
        let body = body_string(resp).await;
        assert!(
            !body.contains(secret),
            "GET /api/config leaked the Telegram credential: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Unlike the terminal token's documented reveal-once (P2), the
    /// credential has no reveal path at all — every render, including
    /// `/settings` immediately after the save that set it, carries at most
    /// its last four characters.
    #[tokio::test]
    async fn settings_page_never_reveals_the_notify_credential_in_full() {
        let dir = fresh_root("terminal-notify-cred-never-full");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());
        let (_token, cookie) = rotate_token(app.clone()).await;

        let secret = "never-shown-in-full-abcd";
        let req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header("content-type", "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie)
            .body(Body::from(format!(
                "notify_chat_id=555&notify_telegram_token={secret}"
            )))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_redirection());

        let resp = get(app, "/settings").await;
        let body = body_string(resp).await;
        assert!(
            !body.contains(secret),
            "/settings rendered the Telegram credential in full: {body}"
        );
        let last_four = &secret[secret.len() - 4..];
        assert!(
            body.contains(last_four),
            "/settings dropped the masked credential entirely: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// agent-terminal-24, the last-mile fix: `update_terminal_config` used to
    /// discard `save_notify_credential`'s `Result` and always redirect to
    /// `?saved=1`, so a user whose credential could not be written was told
    /// it had been. Forces the real failure `config::tests::
    /// a_save_that_cannot_write_reports_the_failure` proves at the config
    /// layer (`crates/mdview-core/src/config.rs`) — no write permission on
    /// the credential directory — through the actual gated route, and checks
    /// every must-have this cell names: the redirect never claims success,
    /// the credential itself never appears in the redirect target, the page
    /// that redirect lands on shows failure rather than the success banner,
    /// and no credential ever reaches disk.
    #[cfg(unix)]
    #[tokio::test]
    async fn credential_save_failure_is_reported_as_failure_not_saved() {
        use std::os::unix::fs::PermissionsExt;

        let dir = fresh_root("terminal-notify-cred-save-fails");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());
        let (_token, cookie) = rotate_token(app.clone()).await;

        let cred_path = mdview_core::config::notify_credential_path_override(Some(&dir));
        let cred_dir = cred_path.parent().unwrap();
        std::fs::create_dir_all(cred_dir).unwrap();
        // No write permission on the credential's own directory: the same
        // failure shape `a_save_that_cannot_write_reports_the_failure`
        // exercises directly against `save_notify_credential`.
        std::fs::set_permissions(cred_dir, std::fs::Permissions::from_mode(0o500)).unwrap();

        let secret = "should-never-reach-disk-or-response";
        let req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header("content-type", "application/x-www-form-urlencoded")
            .header(header::COOKIE, cookie)
            .body(Body::from(format!("notify_telegram_token={secret}")))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();

        // Restore write permission before any further filesystem I/O
        // (cleanup, the follow-up GET below), or later steps fail too.
        std::fs::set_permissions(cred_dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(resp.status().is_redirection());
        let location = resp
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert_ne!(
            location, "/settings?saved=1",
            "a failed credential save must not redirect to the same target a successful save uses"
        );
        assert!(
            !location.contains(secret),
            "the redirect target must never carry the credential: {location}"
        );

        assert_eq!(
            mdview_core::config::load_notify_credential(&cred_path),
            None,
            "a failed save must never leave a partial credential readable"
        );

        let resp = get(app, &location).await;
        let body = body_string(resp).await;
        assert!(
            !body.contains(secret),
            "/settings rendered the Telegram credential after a failed save: {body}"
        );
        assert!(
            !body.contains("fg-banner--success"),
            "a failed credential save must not show the generic success banner: {body}"
        );
        assert!(
            body.contains("could not be saved"),
            "a failed credential save must tell the user it failed: {body}"
        );

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

    /// POSTs `/settings/terminal/token` (a genuine first-run rotation — no
    /// token file exists yet on a fresh scratch dir, so it needs no session),
    /// reads the full token revealed in that one response (P2), then POSTs
    /// `/settings/terminal/login` with it to obtain a real session cookie
    /// through the only function that ever mints one from a presented
    /// credential (`TerminalAuth::verify_and_mint`). Rotation itself no
    /// longer mints a session (agent-terminal-8) — this rotate-then-login
    /// shape is the shared setup every gated-switch test needs to get past
    /// the gate, and it doubles as the regression guard: if any future
    /// change makes the rotate response mint a session again, this helper
    /// would stop needing the login POST at all rather than catching it.
    async fn rotate_token(app: Router) -> (String, String) {
        let rotate_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/terminal/token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rotate_resp.status(), StatusCode::OK);
        assert!(
            rotate_resp.headers().get(header::SET_COOKIE).is_none(),
            "rotation must never itself set a session cookie"
        );
        let body = body_string(rotate_resp).await;
        let marker = "it will not be shown again: <code>";
        let start = body.find(marker).expect("full token banner missing") + marker.len();
        let rest = &body[start..];
        let end = rest.find("</code>").expect("full token banner unterminated");
        let token = rest[..end].to_string();

        let login_resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/terminal/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("token={token}")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            login_resp.status().is_redirection(),
            "login with the just-rotated token must succeed, got {}",
            login_resp.status()
        );
        let set_cookie = login_resp
            .headers()
            .get(header::SET_COOKIE)
            .expect("login response must set the terminal session cookie")
            .to_str()
            .unwrap()
            .to_string();
        let cookie = set_cookie.split(';').next().unwrap().to_string();
        (token, cookie)
    }

    /// A GET request to `/p/{id}/_terminal`, optionally carrying the given
    /// session cookie value (e.g. the one `rotate_token` returns).
    fn terminal_req(id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal"))
            .method("GET");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::empty()).unwrap()
    }

    /// A GET request to `/p/{id}/_terminal/{pane_id}/screen`, optionally
    /// carrying the given session cookie value — the screen sibling of
    /// `terminal_req` above.
    fn terminal_screen_req(id: &str, pane_id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal/{pane_id}/screen"))
            .method("GET");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::empty()).unwrap()
    }

    /// Writes `dir/config.toml` with the D7 `terminal.enabled` switch on —
    /// the gate this cell (agent-terminal-6) adds in front of every route in
    /// the terminal family. Every test below that expects `terminal_page` or
    /// `terminal_screen` to actually run (rather than answer the same opaque
    /// 404 a disabled switch or a missing session both produce) must call
    /// this first; a fresh `build_state_with_dir` config resolves to
    /// `Config::default()`, whose `terminal.enabled` is `false`.
    fn enable_terminal(dir: &Path) {
        let mut cfg = Config::load_from(&dir.join("config.toml"));
        cfg.terminal.enabled = true;
        cfg.save_to(&dir.join("config.toml")).unwrap();
    }

    /// Truth 1: "Without a terminal session the route returns an opaque
    /// 404, identical to an unknown route" — both with no cookie at all and
    /// with a stale/unknown one, and both compared byte-for-byte (status,
    /// headers, body) against a path this router never mounts at all, the
    /// same proof shape `terminal_auth`'s own generic test uses.
    #[tokio::test]
    async fn terminal_route_without_a_session_is_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("terminal-no-session");
        // Enable the terminal so this test isolates the session gate itself
        // (the way its screen sibling,
        // `terminal_screen_without_a_session_is_byte_identical_to_an_unrouted_path`,
        // already does) — without this, the assertion would still pass on a
        // route where `AuthSession` had been deleted entirely, since the
        // disabled-switch check alone already answers 404.
        enable_terminal(&dir);
        let root = fresh_root("terminal-no-session-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "no-session");
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let with_no_session = app
                .clone()
                .oneshot(terminal_req(&project.id, cookie))
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(with_no_session.status(), StatusCode::NOT_FOUND);
            assert_eq!(with_no_session.status(), unrouted.status());
            assert_eq!(
                with_no_session.headers(),
                unrouted.headers(),
                "an unauthenticated /_terminal request must carry no header an unrouted path wouldn't"
            );
            let a = with_no_session.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b, "an unauthenticated /_terminal body leaked something an unrouted path wouldn't");
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth 6 (the carry-over obligation): a wrong-method request to
    /// `GET /p/:id/_terminal` (mounted via `any(...)` + `MethodGate<Get>`,
    /// never `.get(...)`) is byte-identical to a path this router never
    /// mounts, proven against the real router — not just the generic proof
    /// in `terminal_auth.rs`.
    #[tokio::test]
    async fn terminal_route_wrong_method_is_byte_identical_to_unrouted() {
        let dir = fresh_root("terminal-wrong-method");
        let root = fresh_root("terminal-wrong-method-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "wrong-method");
        let app = router(st);

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/p/{}/_terminal", project.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth 6, the other half of the carry-over obligation: applied to
    /// `POST /api/terminal-config` itself, now mounted with `any(...)` +
    /// `MethodGate<Post>` instead of the old `.post(...)` this cell replaces.
    #[tokio::test]
    async fn api_terminal_config_wrong_method_is_byte_identical_to_unrouted() {
        let dir = fresh_root("terminal-config-wrong-method");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let got = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/terminal-config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(got.status(), StatusCode::NOT_FOUND);
        assert_eq!(got.status(), unrouted.status());
        assert_eq!(got.headers(), unrouted.headers());
        let a = got.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A valid session against an unregistered project id gets the ordinary
    /// `not_found` page (same shape as `bee_board`'s own unknown-project
    /// case) — the auth wall is not the only thing standing between a
    /// request and a real 404 for a bad id.
    #[tokio::test]
    async fn terminal_route_unknown_project_is_a_real_not_found_page() {
        let dir = fresh_root("terminal-unknown-project");
        enable_terminal(&dir);
        let st = build_state_with_dir(&dir);
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_req("no-such-project", Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_string(resp).await;
        assert!(body.contains("project not found"), "{body}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D6: the Terminal tab renders on the project home page from the
    /// project id alone — no herdr call, no session — so its presence can
    /// never depend on herdr's state or the viewer's auth.
    #[tokio::test]
    async fn terminal_tab_is_present_on_the_project_home_page() {
        let dir = fresh_root("terminal-tab-present");
        let root = fresh_root("terminal-tab-present-project");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);
        write(&root, ".bee/cells/a.json", &cell_json("c1", "open", &[], "w1"));

        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "tab-present");
        let resp = get(router(st), &format!("/p/{}/", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("href=\"/p/{}/_terminal\"", project.id)),
            "project home page carries no Terminal tab link: {body}"
        );
        assert!(body.contains(">Terminal<"), "{body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// D6: a silent herdr socket renders the named remedy state, never a
    /// raw error and never an empty-panes rendering that would look
    /// identical to a project that genuinely has zero agents.
    #[tokio::test]
    async fn terminal_route_renders_named_remedy_when_herdr_is_silent() {
        let dir = fresh_root("terminal-herdr-down");
        let root = fresh_root("terminal-herdr-down-project");
        enable_terminal(&dir);
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        fake.set_available(false);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "herdr-down");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("herdr is not running"),
            "no named remedy in the down state: {body}"
        );
        assert!(
            !body.contains("No agents are running under this project"),
            "the down state must never render as an ordinary empty-panes list: {body}"
        );
        assert!(
            !body.to_lowercase().contains("herdrerror"),
            "the down state leaked a raw error type into the page: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// D2's containment boundary, all four named cases in one project: a
    /// pane exactly at the root (included), one directory above (excluded),
    /// one directory below (included), and one whose *raw* cwd sits inside
    /// the root but is a symlink resolving outside it (excluded — proves the
    /// join runs the boundary's real symlink-resolving check, never a text
    /// prefix comparison). A fifth pane under a second, unrelated project's
    /// root proves cross-project isolation in the same request.
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_route_lists_only_panes_within_the_project_root_boundary() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("terminal-boundary-data");
        enable_terminal(&dir);
        let scratch = fresh_root("terminal-boundary-scratch");
        let root_a = scratch.join("project-a");
        let root_b = scratch.join("project-b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        let below = root_a.join("sub");
        std::fs::create_dir_all(&below).unwrap();
        let escape_target = scratch.join("outside-a");
        std::fs::create_dir_all(&escape_target).unwrap();
        let symlink_path = root_a.join("escape-link");
        std::os::unix::fs::symlink(&escape_target, &symlink_path).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let at_root = fake
            .agent_start("w1", Some(&root_a.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let above = fake
            .agent_start(
                "w1",
                Some(&scratch.to_string_lossy()), // project-a's parent directory
                &["claude".to_string()],
            )
            .await
            .unwrap();
        let below_agent = fake
            .agent_start("w1", Some(&below.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let via_symlink = fake
            .agent_start(
                "w1",
                Some(&symlink_path.to_string_lossy()), // raw cwd is under root_a, resolves outside
                &["claude".to_string()],
            )
            .await
            .unwrap();
        let other_project = fake
            .agent_start("w1", Some(&root_b.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project_a = register(&st, &root_a, "project-a");
        let project_b = register(&st, &root_b, "project-b");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp_a = app
            .clone()
            .oneshot(terminal_req(&project_a.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp_a.status(), StatusCode::OK);
        let body_a = body_string(resp_a).await;
        assert!(body_a.contains(&at_root.name), "root pane missing: {body_a}");
        assert!(body_a.contains(&below_agent.name), "below-root pane missing: {body_a}");
        assert!(!body_a.contains(&above.name), "above-root pane leaked in: {body_a}");
        assert!(
            !body_a.contains(&via_symlink.name),
            "symlink-escape pane leaked in: {body_a}"
        );
        assert!(
            !body_a.contains(&other_project.name),
            "project-b's pane leaked into project-a's list: {body_a}"
        );

        let resp_b = app
            .oneshot(terminal_req(&project_b.id, Some(&cookie)))
            .await
            .unwrap();
        let body_b = body_string(resp_b).await;
        assert!(
            body_b.contains(&other_project.name),
            "project-b's own pane missing from its own list: {body_b}"
        );
        assert!(
            !body_b.contains(&at_root.name),
            "project-a's pane leaked into project-b's list: {body_b}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// agent-terminal-6, truth 2: "Without a terminal session the screen
    /// endpoint returns an opaque 404" — the same byte-identical-to-unrouted
    /// proof `terminal_route_without_a_session_is_byte_identical_to_an_unrouted_path`
    /// uses for the page, applied to its screen sibling.
    #[tokio::test]
    async fn terminal_screen_without_a_session_is_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("terminal-screen-no-session");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-no-session-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "screen-no-session");
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let with_no_session = app
                .clone()
                .oneshot(terminal_screen_req(&project.id, "w1:p1", cookie))
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(with_no_session.status(), StatusCode::NOT_FOUND);
            assert_eq!(with_no_session.status(), unrouted.status());
            assert_eq!(with_no_session.headers(), unrouted.headers());
            let a = with_no_session.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The screen route's own method-mismatch-oracle proof, mirroring
    /// `terminal_route_wrong_method_is_byte_identical_to_unrouted` — mounted
    /// with `any(...)` + `MethodGate<Get>`, never `.get(...)`.
    #[tokio::test]
    async fn terminal_screen_wrong_method_is_byte_identical_to_unrouted() {
        let dir = fresh_root("terminal-screen-wrong-method");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-wrong-method-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "screen-wrong-method");
        let app = router(st);

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/p/{}/_terminal/w1:p1/screen", project.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The must-have this cell's carried-over deviation exists to close:
    /// with the D7 `terminal.enabled` switch off, both `terminal_page` and
    /// `terminal_screen` answer exactly as an unrouted path would — status,
    /// headers and body — even with a session a valid rotation just minted.
    #[tokio::test]
    async fn terminal_family_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session() {
        let dir = fresh_root("terminal-family-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("terminal-family-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "family-disabled");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted_status = unrouted.status();
        let unrouted_headers = unrouted.headers().clone();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

        let page = app
            .clone()
            .oneshot(terminal_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(page.status(), unrouted_status);
        assert_eq!(page.headers(), &unrouted_headers);
        let page_body = page.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            page_body, unrouted_body,
            "the terminal page must be unreachable while the switch is off, even with a valid session"
        );

        let screen = app
            .oneshot(terminal_screen_req(&project.id, "w1:p1", Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(screen.status(), unrouted_status);
        assert_eq!(screen.headers(), &unrouted_headers);
        let screen_body = screen.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            screen_body, unrouted_body,
            "the screen endpoint must be unreachable while the switch is off, even with a valid session"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The other half of the same must-have: the settings page and the
    /// gated `POST /api/terminal-config` switch endpoint must both stay
    /// reachable while the switch is off — otherwise it could never be
    /// turned back on.
    #[tokio::test]
    async fn settings_and_terminal_config_switch_stay_reachable_while_terminal_disabled() {
        let dir = fresh_root("terminal-disabled-settings-reachable");
        // The switch defaults off; nothing in this test turns it on before
        // exercising the two routes that must stay reachable regardless.
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let settings = get(app.clone(), "/settings").await;
        assert_eq!(settings.status(), StatusCode::OK);

        let (_token, cookie) = rotate_token(app.clone()).await;

        let switch_resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header(header::COOKIE, cookie)
                    .body(Body::from("enabled=on"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            switch_resp.status().is_redirection(),
            "the gated switch endpoint must stay reachable while the terminal switch is off, got {}",
            switch_resp.status()
        );

        let saved = Config::load_from(&dir.join("config.toml"));
        assert!(
            saved.terminal.enabled,
            "the switch endpoint did not actually turn the switch on"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// agent-terminal-6, truth: "A screen read for a pane outside the
    /// project's root is refused, even with a valid session" — the D2
    /// containment boundary applied to the screen route itself, not only the
    /// page's pane list.
    #[tokio::test]
    async fn terminal_screen_refuses_a_pane_outside_the_project_root() {
        let dir = fresh_root("terminal-screen-boundary");
        enable_terminal(&dir);
        let scratch = fresh_root("terminal-screen-boundary-scratch");
        let root_a = scratch.join("project-a");
        let outside = scratch.join("outside-a");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let outside_agent = fake
            .agent_start("w1", Some(&outside.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project_a = register(&st, &root_a, "project-a-screen");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_screen_req(&project_a.id, &outside_agent.pane_id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "a pane outside the project root must be refused, not read"
        );
        let body = body_string(resp).await;
        assert!(body.contains("pane not found"), "{body}");
        assert!(
            !body.contains(&outside_agent.name),
            "the refused pane's own screen must never be echoed back: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// A pane id that never resolves to any pane in this project's own
    /// boundary-filtered list — never a crash, never treated as the herdr-down
    /// state, just the ordinary not-found.
    #[tokio::test]
    async fn terminal_screen_unknown_pane_in_project_is_not_found() {
        let dir = fresh_root("terminal-screen-unknown-pane");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-unknown-pane-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "screen-unknown-pane");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_screen_req(&project.id, "no-such-pane", Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_string(resp).await;
        assert!(body.contains("pane not found"), "{body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-6, truth: "The rendered screen shows the text herdr
    /// returned, with a UTF-8 screen containing wide CJK and emoji intact" —
    /// proven on the JSON the client polls. Since agent-terminal-12, `text`
    /// carries `mdview_core::ansi::to_html`'s output rather than the raw
    /// string; this screen carries no ANSI codes and no HTML metacharacters,
    /// so translation is the identity here — `ansi.rs`'s own tests cover the
    /// colour/attribute/escaping cases. Also pins the response shape against
    /// herdr-go's `ScreenBody { text, revision }`.
    #[tokio::test]
    async fn terminal_screen_returns_the_panes_text_and_revision_with_utf8_intact() {
        let dir = fresh_root("terminal-screen-utf8");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-utf8-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let screen_text = "屏幕内容 😀\n❯ ";
        fake.seed_scroll_pane(&started.pane_id, screen_text, screen_text, None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "screen-utf8");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["text"], serde_json::json!(screen_text), "{body}");
        assert_eq!(json["revision"], serde_json::json!(1), "{body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-12, truth: "A screen carrying SGR colour, bold and
    /// inverse sequences renders as styled markup, not as literal escape
    /// characters" and "HTML metacharacters in the screen text are escaped,
    /// and no unrecognised escape sequence is ever emitted into the page" —
    /// proven at the HTTP layer, not only inside `ansi.rs`'s own unit tests,
    /// so a future regression that stops the endpoint from calling the
    /// translator at all is caught here.
    #[tokio::test]
    async fn terminal_screen_translates_ansi_into_safe_html_through_the_real_endpoint() {
        let dir = fresh_root("terminal-screen-ansi");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-ansi-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let raw = "\u{1b}[31m<script>bad</script>\u{1b}[0m\u{1b}[2Jplain";
        fake.seed_scroll_pane(&started.pane_id, raw, raw, None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "screen-ansi");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = json["text"].as_str().unwrap();
        assert_eq!(
            text,
            mdview_core::ansi::to_html(raw),
            "the screen endpoint must return mdview-core's ansi translation verbatim: {text}"
        );
        assert!(text.contains("ansi-fg-red"), "no colour markup: {text}");
        assert!(!text.contains("<script>"), "raw script tag leaked: {text}");
        assert!(text.contains("&lt;script&gt;"), "text must still be escaped: {text}");
        assert!(text.ends_with("plain"), "cursor-clear escape must be dropped, not shown: {text}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-6, truth: "herdr going silent between polls surfaces
    /// the named remedy rather than a blank screen" — the server-side half:
    /// a silent socket never answers the poll with `2xx`, and the body
    /// carries the same "herdr is not running" wording `terminal_down_page`
    /// renders, so `assets/app.js` (which shows that literal text on any
    /// non-OK response) and the page agree.
    #[tokio::test]
    async fn terminal_screen_reports_herdr_down_without_a_success_status() {
        let dir = fresh_root("terminal-screen-herdr-down");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-herdr-down-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        fake.set_available(false);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "screen-herdr-down");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_screen_req(&project.id, "w1:p1", Some(&cookie)))
            .await
            .unwrap();
        assert!(
            !resp.status().is_success(),
            "a silent herdr socket must never answer the screen poll with 2xx"
        );
        let body = body_string(resp).await;
        assert!(
            body.contains("herdr is not running"),
            "no named remedy in the screen endpoint's herdr-down answer: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    // -----------------------------------------------------------------
    // agent-terminal-16 (D9): the Transcript tab and its data endpoint.
    // -----------------------------------------------------------------

    /// A GET request to `/p/{id}/_transcript`, the Transcript tab's page
    /// route — the transcript sibling of `terminal_req`.
    fn transcript_page_req(id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_transcript"))
            .method("GET");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::empty()).unwrap()
    }

    /// A GET request to `/p/{id}/_terminal/{pane_id}/transcript`, optionally
    /// carrying a `cursor` query param and a session cookie — the transcript
    /// sibling of `terminal_screen_req`.
    fn terminal_transcript_req(
        id: &str,
        pane_id: &str,
        cursor: Option<&str>,
        cookie: Option<&str>,
    ) -> Request<Body> {
        let mut uri = format!("/p/{id}/_terminal/{pane_id}/transcript");
        if let Some(c) = cursor {
            uri.push_str("?cursor=");
            uri.push_str(&urlencoding_lite(c));
        }
        let mut b = Request::builder().uri(uri).method("GET");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::empty()).unwrap()
    }

    /// Minimal query-string escaping for the handful of characters
    /// `cursor_never_escapes_the_project_dir`-style test fixtures actually
    /// carry (`:`, `/`, `.`, spaces) — this test module has no HTTP client
    /// crate with a real URL encoder, and `cursor`'s own alphabet
    /// (`<file>.jsonl:<offset>`) needs only these escaped to survive as one
    /// query value.
    fn urlencoding_lite(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                ':' => "%3A".to_string(),
                '/' => "%2F".to_string(),
                ' ' => "%20".to_string(),
                other => other.to_string(),
            })
            .collect()
    }

    /// Writes one raw JSONL transcript line for `cwd`'s Claude Code project
    /// directory under a fresh `transcript_root` — the fixture every
    /// transcript route test builds on. `session` is the file stem (e.g.
    /// `"s1"`); the caller passes complete `{"type": ...}\n`-shaped lines in
    /// `body`.
    fn write_transcript(transcript_root: &Path, cwd: &str, session: &str, body: &str) {
        let dir = transcript_root.join(mdview_core::transcript::encode_project_dir(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{session}.jsonl")), body).unwrap();
    }

    /// A pane's cwd as `terminal_transcript` itself resolves it: the same
    /// `std::fs::canonicalize` call `mdview_core::paths_boundary::Boundary::
    /// validate_existing` makes — so a fixture built from this value always
    /// lands in the directory the route under test actually reads, even in
    /// an environment where `std::env::temp_dir()` itself is a symlink.
    fn canonical_cwd(root: &Path) -> String {
        std::fs::canonicalize(root)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    /// Truth: "Without a session, with a wrong method, and with the terminal
    /// switch off, the transcript endpoint answers byte-identically to an
    /// unrouted path" — the no-session third, proven the same
    /// byte-identical-to-unrouted way `terminal_screen`'s own sibling test
    /// does. Verified by temporarily deleting the `_session:
    /// terminal_auth::AuthSession` extractor from `terminal_transcript` and
    /// re-running this test: it went red (200, not 404) before the guard was
    /// restored.
    #[tokio::test]
    async fn terminal_transcript_without_a_session_is_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("transcript-no-session");
        enable_terminal(&dir);
        let root = fresh_root("transcript-no-session-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "transcript-no-session");
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let with_no_session = app
                .clone()
                .oneshot(terminal_transcript_req(&project.id, "w1:p1", None, cookie))
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(with_no_session.status(), StatusCode::NOT_FOUND);
            assert_eq!(with_no_session.status(), unrouted.status());
            assert_eq!(with_no_session.headers(), unrouted.headers());
            let a = with_no_session.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The transcript endpoint's own method-mismatch-oracle proof, mirroring
    /// `terminal_screen_wrong_method_is_byte_identical_to_unrouted` — mounted
    /// with `any(...)` + `MethodGate<Get>`, never `.get(...)`. Carries no
    /// session and no enabled switch, so on its own this does **not**
    /// isolate `MethodGate`: `AuthSession`'s own rejection already answers
    /// the same opaque 404 even with `MethodGate` deleted entirely — see
    /// `terminal_transcript_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal`
    /// below for the test that actually goes red on that guard alone
    /// (the toothless-assertion shape this repo's own review found
    /// elsewhere in this feature, `docs/history/learnings/
    /// 20260805-toothless-security-assertions.md`).
    #[tokio::test]
    async fn terminal_transcript_wrong_method_is_byte_identical_to_unrouted() {
        let dir = fresh_root("transcript-wrong-method");
        let root = fresh_root("transcript-wrong-method-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "transcript-wrong-method");
        let app = router(st);

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/p/{}/_terminal/w1:p1/transcript", project.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth (the carried-over method-mismatch-oracle obligation, isolated,
    /// mirroring `terminal_route_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal`):
    /// at least one wrong-method test must fail if `MethodGate` were removed
    /// while a valid session and an enabled terminal are already in place.
    /// The test above carries no session and the switch off, so it would
    /// still pass unchanged even with `MethodGate` deleted entirely —
    /// `AuthSession`'s own rejection answers the same opaque 404 first,
    /// without `MethodGate` ever mattering (confirmed: deleting only
    /// `_method: MethodGate<Get>` there left it green). This test instead
    /// posts against a *known, in-boundary* pane (`started.pane_id`) with a
    /// freshly rotated session and the switch on — every other guard
    /// satisfied — so `MethodGate` alone stands between the POST and the
    /// handler body. Verified by temporarily deleting `_method:
    /// MethodGate<Get>` from `terminal_transcript` and re-running this test:
    /// it went red (`200`, the transcript's ordinary `available: false`
    /// JSON, not `404`) before the guard was restored.
    #[tokio::test]
    async fn terminal_transcript_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal()
    {
        let dir = fresh_root("transcript-wrong-method-isolated");
        enable_terminal(&dir);
        let root = fresh_root("transcript-wrong-method-isolated-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "wrong-method-isolated");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/p/{}/_terminal/{}/transcript", project.id, started.pane_id))
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The must-have's third leg: with the D7 `terminal.enabled` switch off,
    /// both the Transcript tab's page and its data endpoint answer exactly
    /// as an unrouted path would, even with a session a valid rotation just
    /// minted — mirroring
    /// `terminal_family_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session`.
    /// Verified by temporarily deleting the `if !terminal_family_enabled(&st)`
    /// check from `terminal_transcript` and re-running: it went red (200
    /// `available: false`, not 404) before the guard was restored.
    #[tokio::test]
    async fn transcript_family_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session() {
        let dir = fresh_root("transcript-family-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("transcript-family-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "transcript-family-disabled");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted_status = unrouted.status();
        let unrouted_headers = unrouted.headers().clone();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

        let page = app
            .clone()
            .oneshot(transcript_page_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(page.status(), unrouted_status);
        assert_eq!(page.headers(), &unrouted_headers);
        let page_body = page.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            page_body, unrouted_body,
            "the Transcript tab must be unreachable while the switch is off, even with a valid session"
        );

        let data = app
            .oneshot(terminal_transcript_req(&project.id, "w1:p1", None, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(data.status(), unrouted_status);
        assert_eq!(data.headers(), &unrouted_headers);
        let data_body = data.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            data_body, unrouted_body,
            "the transcript endpoint must be unreachable while the switch is off, even with a valid session"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: "Reading a transcript for a pane outside the project's root is
    /// refused, even with a valid session" — the D2 containment boundary
    /// applied to the transcript route itself, mirroring
    /// `terminal_screen_refuses_a_pane_outside_the_project_root`. Verified by
    /// temporarily replacing `project_pane_cwd_in_boundary`'s boundary
    /// construction with an always-true membership check and re-running:
    /// it went red (200, not 404) before the guard was restored.
    #[tokio::test]
    async fn terminal_transcript_refuses_a_pane_outside_the_project_root() {
        let dir = fresh_root("transcript-boundary");
        enable_terminal(&dir);
        let scratch = fresh_root("transcript-boundary-scratch");
        let root_a = scratch.join("project-a");
        let outside = scratch.join("outside-a");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let outside_agent = fake
            .agent_start("w1", Some(&outside.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project_a = register(&st, &root_a, "project-a-transcript");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_transcript_req(
                &project_a.id,
                &outside_agent.pane_id,
                None,
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "a pane outside the project root must be refused, not read"
        );
        let body = body_string(resp).await;
        assert!(body.contains("pane not found"), "{body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Truth: "A pane whose agent has written no transcript yet shows a
    /// named state, not an empty frame" — the route-level half: no
    /// `~/.claude/projects/<encoded-cwd>` directory for this pane's cwd
    /// answers `200` with `available: false`, never the herdr-down error
    /// shape and never a bare empty `lines: []`.
    #[tokio::test]
    async fn terminal_transcript_reports_not_available_when_no_transcript_file_exists() {
        let dir = fresh_root("transcript-not-available");
        enable_terminal(&dir);
        let root = fresh_root("transcript-not-available-project");
        let transcript_root = fresh_root("transcript-not-available-claude");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        st.transcript_root = Some(transcript_root.clone());
        let project = register(&st, &root, "transcript-not-available");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_transcript_req(&project.id, &started.pane_id, None, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["available"], serde_json::json!(false), "{body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&transcript_root).ok();
    }

    /// Truth: "Successive reads with the returned cursor produce no
    /// duplicated and no skipped records" — proven at the HTTP layer: an
    /// opening read (no cursor) backfills one record, a second read with the
    /// returned cursor sees only what was appended after it (no duplicate of
    /// the first record), and a third read with *that* cursor is empty (no
    /// record silently skipped by re-reading past it).
    #[tokio::test]
    async fn terminal_transcript_cursor_reads_produce_no_duplication_or_skip() {
        let dir = fresh_root("transcript-cursor");
        enable_terminal(&dir);
        let root = fresh_root("transcript-cursor-project");
        let transcript_root = fresh_root("transcript-cursor-claude");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let cwd = canonical_cwd(&root);
        write_transcript(
            &transcript_root,
            &cwd,
            "s1",
            "{\"type\":\"user\",\"message\":{\"content\":\"first prompt\"}}\n",
        );

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        st.transcript_root = Some(transcript_root.clone());
        let project = register(&st, &root, "transcript-cursor");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        // Opening read: backfills the tail.
        let open_resp = app
            .clone()
            .oneshot(terminal_transcript_req(&project.id, &started.pane_id, None, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(open_resp.status(), StatusCode::OK);
        let open_body = body_string(open_resp).await;
        let open_json: serde_json::Value = serde_json::from_str(&open_body).unwrap();
        assert_eq!(open_json["available"], serde_json::json!(true), "{open_body}");
        let open_lines: Vec<String> = open_json["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(open_lines, vec!["» first prompt".to_string()], "{open_body}");
        let cursor1 = open_json["cursor"].as_str().unwrap().to_string();

        // Append a second record after the cursor was minted.
        let claude_dir = transcript_root.join(mdview_core::transcript::encode_project_dir(&cwd));
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(claude_dir.join("s1.jsonl"))
            .unwrap();
        use std::io::Write as _;
        writeln!(f, "{{\"type\":\"user\",\"message\":{{\"content\":\"second prompt\"}}}}").unwrap();
        drop(f);

        // Poll with the first cursor: only the new record, never the first
        // one again.
        let mid_resp = app
            .clone()
            .oneshot(terminal_transcript_req(
                &project.id,
                &started.pane_id,
                Some(&cursor1),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(mid_resp.status(), StatusCode::OK);
        let mid_body = body_string(mid_resp).await;
        let mid_json: serde_json::Value = serde_json::from_str(&mid_body).unwrap();
        let mid_lines: Vec<String> = mid_json["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            mid_lines,
            vec!["» second prompt".to_string()],
            "the first record must never be re-delivered: {mid_body}"
        );
        let cursor2 = mid_json["cursor"].as_str().unwrap().to_string();
        assert_ne!(cursor1, cursor2, "the cursor must advance past the newly read record");

        // Poll again with the latest cursor: nothing new, nothing skipped.
        let after_resp = app
            .oneshot(terminal_transcript_req(
                &project.id,
                &started.pane_id,
                Some(&cursor2),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(after_resp.status(), StatusCode::OK);
        let after_body = body_string(after_resp).await;
        let after_json: serde_json::Value = serde_json::from_str(&after_body).unwrap();
        assert_eq!(
            after_json["lines"].as_array().unwrap().len(),
            0,
            "a fully-caught-up poll must return no records: {after_body}"
        );
        assert_eq!(after_json["cursor"].as_str().unwrap(), cursor2);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&transcript_root).ok();
    }

    /// A malformed `cursor` (rejected by `mdview_core::transcript`'s own
    /// `parse_cursor` guard, agent-terminal-15) is answered `400`, not a
    /// crash and not silently treated as "no cursor" — this is the second
    /// line the cell's action names, never a substitute for the D2 boundary
    /// check above it.
    #[tokio::test]
    async fn terminal_transcript_bad_cursor_is_a_bad_request() {
        let dir = fresh_root("transcript-bad-cursor");
        enable_terminal(&dir);
        let root = fresh_root("transcript-bad-cursor-project");
        let transcript_root = fresh_root("transcript-bad-cursor-claude");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let cwd = canonical_cwd(&root);
        write_transcript(&transcript_root, &cwd, "s1", "");

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        st.transcript_root = Some(transcript_root.clone());
        let project = register(&st, &root, "transcript-bad-cursor");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_transcript_req(
                &project.id,
                &started.pane_id,
                Some("../../etc/passwd.jsonl:0"),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&transcript_root).ok();
    }

    /// Truth: "Without a session, the Transcript *page* route returns an
    /// opaque 404, identical to an unknown route" — with the D7 switch
    /// already ON, so this isolates the session gate itself rather than the
    /// disabled-switch check (the trap this feature has already fallen into
    /// three times: a no-session test with the switch left off passes on the
    /// switch, not on `AuthSession` — see
    /// `docs/history/learnings/20260805-toothless-security-assertions.md`).
    /// Every other test that reaches `transcript_page`
    /// (`transcript_page_renders_the_tab_and_a_viewport_per_pane`,
    /// `transcript_family_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session`)
    /// carries a valid cookie, so none of them would go red if
    /// `_session: terminal_auth::AuthSession` were deleted from
    /// `transcript_page` — this one does. Mirrors
    /// `terminal_route_without_a_session_is_byte_identical_to_an_unrouted_path`
    /// (the Terminal tab's own page-route sibling proof). Verified by
    /// temporarily deleting `_session: terminal_auth::AuthSession` from
    /// `transcript_page` and re-running: it went red (200, not 404) before
    /// the guard was restored.
    #[tokio::test]
    async fn transcript_page_without_a_session_is_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("transcript-page-no-session");
        enable_terminal(&dir);
        let root = fresh_root("transcript-page-no-session-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "transcript-page-no-session");
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let with_no_session = app
                .clone()
                .oneshot(transcript_page_req(&project.id, cookie))
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(with_no_session.status(), StatusCode::NOT_FOUND);
            assert_eq!(with_no_session.status(), unrouted.status());
            assert_eq!(with_no_session.headers(), unrouted.headers());
            let a = with_no_session.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth (the carried-over method-mismatch-oracle obligation, isolated,
    /// mirroring
    /// `terminal_route_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal`):
    /// at least one wrong-method test against the Transcript *page* route
    /// must fail if `MethodGate` were removed from `transcript_page` while a
    /// valid session and an enabled terminal are already in place. A
    /// no-session/switch-off wrong-method test would still pass unchanged
    /// even with `MethodGate` deleted entirely, since `AuthSession`'s own
    /// rejection (or the disabled switch) answers the same opaque 404 first
    /// without `MethodGate` ever mattering — this test instead POSTs with a
    /// freshly rotated session and the switch on, so `MethodGate` alone
    /// stands between the POST and the handler body. Verified by temporarily
    /// deleting `_method: terminal_auth::MethodGate<terminal_auth::Get>` from
    /// `transcript_page` and re-running this test: it went red (200, the
    /// rendered Transcript page, not 404) before the guard was restored.
    #[tokio::test]
    async fn transcript_page_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal()
    {
        let dir = fresh_root("transcript-page-wrong-method-isolated");
        enable_terminal(&dir);
        let root = fresh_root("transcript-page-wrong-method-isolated-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "transcript-page-wrong-method-isolated");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/p/{}/_transcript", project.id))
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: "The Transcript tab is a separate tab from Terminal on a
    /// project page" — the page route renders the nav link and one
    /// `.term-transcript` viewport per pane, distinct from `.term-screen`.
    #[tokio::test]
    async fn transcript_page_renders_the_tab_and_a_viewport_per_pane() {
        let dir = fresh_root("transcript-page-render");
        enable_terminal(&dir);
        let root = fresh_root("transcript-page-render-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "transcript-page-render");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(transcript_page_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("href=\"/p/{}/_transcript\"", project.id)),
            "no Transcript tab link: {body}"
        );
        assert!(
            body.contains(&format!(
                "class=\"term-transcript\" data-pane-id=\"{}\"",
                started.pane_id
            )),
            "no transcript viewport for the pane: {body}"
        );
        assert!(!body.contains("class=\"term-screen\""), "the Transcript tab must not render a screen viewport: {body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-20/22, truth: "A poll whose body is still arriving when
    /// the next tick fires cannot cause a record to be appended twice" — a
    /// grep-based proof over the vendored client source `views::APP_JS` is
    /// served from (`assets/app.js`), the same shape
    /// `typed_text_and_named_keys_never_appear_in_a_tracing_call` above uses
    /// for a client-file guarantee `cargo test` can otherwise only assert on
    /// as a string, since there is no JS test runner in this workspace.
    ///
    /// agent-terminal-20's original version of this test only checked that
    /// `"inFlight[paneId] = false;"` appears *somewhere* in the file. That
    /// string occurs twice by design (the success settle and the failure
    /// `.catch`), so it stayed green even with agent-terminal-20's actual
    /// bug in place — clearing the flag in the *headers* handler, before the
    /// cursor below had advanced, which reopens the exact double-append the
    /// flag exists to prevent (see docs/history/learnings/
    /// 20260805-toothless-security-assertions.md: a bare `contains` proves
    /// nothing about *where* a match sits). This version pins each clear to
    /// its specific block instead of the string in isolation.
    #[test]
    fn transcript_poller_clears_the_in_flight_flag_only_once_the_request_has_settled() {
        let js = views::APP_JS;
        assert!(
            js.contains("if (inFlight[paneId]) return;"),
            "the transcript poller must skip a tick while a poll for that pane is already outstanding"
        );
        assert!(
            js.contains("inFlight[paneId] = true;"),
            "the transcript poller must mark a pane in-flight before dispatching its fetch"
        );

        // The headers handler (the first `.then`, keyed by its own comment)
        // must not clear the flag — clearing there, before the cursor below
        // has advanced, is exactly the bug this test exists to catch.
        let headers_start = js
            .find("// Headers have arrived, but the request has not settled")
            .expect("the headers-handler comment documenting why it must not clear the flag must exist");
        let headers_end = js[headers_start..]
            .find("return res.json();")
            .map(|i| headers_start + i)
            .expect("the headers handler must return the parsed body");
        let headers_block = &js[headers_start..headers_end];
        assert!(
            !headers_block.contains("inFlight[paneId] = false;"),
            "the in-flight flag must not clear on headers alone, before the cursor has \
             advanced — a tick landing in that window would refetch with the same cursor \
             and double-append: {headers_block}"
        );

        // The cursor must advance, and only then may the flag clear on the
        // success path — enforced by requiring the clear to appear in the
        // source *after* the cursor assignment (the promise chain settles
        // its `.then` callbacks strictly in that order).
        let cursor_idx = js
            .find("cursors[paneId] = body.cursor;")
            .expect("the cursor must advance on a successful poll");
        let settle_idx = js[cursor_idx..]
            .find("inFlight[paneId] = false;")
            .map(|i| cursor_idx + i);
        assert!(
            settle_idx.is_some(),
            "the success path must clear the in-flight flag after the cursor advances"
        );

        // The failure path must independently clear the flag too — deleting
        // just this clear (leaving the success-path clear alone) is exactly
        // the case a plain `contains` on the flag-clear string cannot catch,
        // since the success-path clear still matches. Pin the check to the
        // failure path's own comment so this can only pass if the clear
        // lives inside that specific block.
        let catch_start = js
            .find("// The request failed outright (network error")
            .expect("the failure-path catch comment must exist");
        let catch_end = js[catch_start..]
            .find("function pollAll(")
            .map(|i| catch_start + i)
            .expect("pollAll must follow the transcript poller's pollOne");
        let catch_block = &js[catch_start..catch_end];
        assert!(
            catch_block.contains("inFlight[paneId] = false;"),
            "the failure path must clear the in-flight flag too, or one error wedges this \
             pane's poller forever: {catch_block}"
        );
    }

    /// agent-terminal-20/22, truth: "Swapping the session-expired and
    /// transient-error states makes a test fail" — same grep-based-over-
    /// `views::APP_JS` proof shape as the in-flight test above, but pinned to
    /// each branch's *specific* constant rather than checking the two
    /// constants exist and their declaration lines differ.
    ///
    /// agent-terminal-20's original version asserted only that a status
    /// check (`res.status === 404`) appears somewhere, and that the two
    /// `var SESSION_EXPIRED_TEXT` / `var TRANSCRIPT_ERROR_TEXT` declaration
    /// lines are textually unequal. Neither assertion reads which constant
    /// is actually used inside which branch, so swapping the two
    /// `showState(...)` calls between the 404 branch and the non-ok branch —
    /// showing the wrong state for each failure kind — left both assertions
    /// green (see docs/history/learnings/20260805-toothless-security-assertions.md:
    /// an assertion must be checked against the value that would actually
    /// leak/change, not a nearby literal). This version extracts each
    /// branch's own source and requires it to name the one constant it must
    /// use and not the other.
    #[test]
    fn transcript_poller_distinguishes_session_expired_from_a_transient_error() {
        let js = views::APP_JS;
        assert!(
            js.contains("SESSION_EXPIRED_TEXT"),
            "the transcript poller must name a session-expired state"
        );
        assert!(
            js.contains("TRANSCRIPT_ERROR_TEXT"),
            "the transcript poller must name a transient-error state, distinct from session-expired"
        );

        let status_check = js
            .find("if (res.status === 404) {")
            .expect("the 404 branch must exist");
        let ok_check = js[status_check..]
            .find("if (!res.ok) {")
            .map(|i| status_check + i)
            .expect("the non-ok branch must exist");
        let after_ok = js[ok_check..]
            .find("return res.json();")
            .map(|i| ok_check + i)
            .expect("the branch that parses the successful body must exist");

        let session_expired_branch = &js[status_check..ok_check];
        let transient_error_branch = &js[ok_check..after_ok];

        assert!(
            session_expired_branch.contains("SESSION_EXPIRED_TEXT"),
            "a 404 (the opaque answer to every guard failure, D4) must show the \
             session-expired state: {session_expired_branch}"
        );
        assert!(
            !session_expired_branch.contains("TRANSCRIPT_ERROR_TEXT"),
            "a 404 must not also show the transient-error state, or the two states could be \
             swapped and this test would not notice: {session_expired_branch}"
        );
        assert!(
            transient_error_branch.contains("TRANSCRIPT_ERROR_TEXT"),
            "a non-404, non-ok response must show the transient-error state: {transient_error_branch}"
        );
        assert!(
            !transient_error_branch.contains("SESSION_EXPIRED_TEXT"),
            "a non-404, non-ok response must not also show the session-expired state, or the \
             two states could be swapped and this test would not notice: {transient_error_branch}"
        );

        // A network failure (the `.catch`) must also read as transient, not
        // as session-expired — the reader should not be told to re-auth just
        // because the request never reached the server at all.
        let catch_start = js
            .find("// The request failed outright (network error")
            .expect("the failure-path catch comment must exist");
        let catch_end = js[catch_start..]
            .find("function pollAll(")
            .map(|i| catch_start + i)
            .expect("pollAll must follow the transcript poller's pollOne");
        let catch_block = &js[catch_start..catch_end];
        assert!(
            catch_block.contains("TRANSCRIPT_ERROR_TEXT"),
            "a network failure must show the transient-error state: {catch_block}"
        );
        assert!(
            !catch_block.contains("SESSION_EXPIRED_TEXT"),
            "a network failure must not show the session-expired state: {catch_block}"
        );
    }

    /// A POST request to `/p/{id}/_terminal/{pane_id}/input` carrying a JSON
    /// `{ "text": ..., "submit": ... }` body (the `submit` key omitted
    /// entirely when `submit` is `None`, proving the real absent-flag case —
    /// not just a JSON `false`), optionally carrying the given session
    /// cookie value.
    fn terminal_input_req(
        id: &str,
        pane_id: &str,
        text: &str,
        submit: Option<bool>,
        cookie: Option<&str>,
    ) -> Request<Body> {
        let body = match submit {
            Some(s) => serde_json::json!({ "text": text, "submit": s }),
            None => serde_json::json!({ "text": text }),
        };
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal/{pane_id}/input"))
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    /// A POST request to `/p/{id}/_terminal/{pane_id}/keys` carrying a JSON
    /// `{ "keys": [...] }` body, optionally carrying the given session
    /// cookie value — the keys sibling of `terminal_input_req`.
    fn terminal_keys_req(
        id: &str,
        pane_id: &str,
        keys: &[&str],
        cookie: Option<&str>,
    ) -> Request<Body> {
        let body = serde_json::json!({ "keys": keys });
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal/{pane_id}/keys"))
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    /// agent-terminal-9, truth: "Typed text reaches the pane's input without
    /// being submitted when the submit flag is absent" — posted with no
    /// `submit` key at all (the real omitted-flag case), the text lands in
    /// the pane's screen but no Enter follows it.
    #[tokio::test]
    async fn terminal_input_without_submit_stages_text_but_never_submits_it() {
        let dir = fresh_root("terminal-input-stage");
        enable_terminal(&dir);
        let root = fresh_root("terminal-input-stage-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "input-stage");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .clone()
            .oneshot(terminal_input_req(
                &project.id,
                &started.pane_id,
                "draft reply",
                None,
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ok_body = body_string(resp).await;
        assert!(ok_body.contains("\"ok\":true"), "{ok_body}");

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, Some(&cookie)))
            .await
            .unwrap();
        let body = body_string(screen_resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = json["text"].as_str().unwrap();
        assert!(text.contains("draft reply"), "{text}");
        assert!(
            !text.ends_with('\n'),
            "an omitted submit flag must never press Enter: {text:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-9, truth: "With the submit flag set, the text is sent
    /// and the Enter keypress is a separate call after it" — the pane's
    /// screen carries the text followed by the newline `Herdr::send_input`
    /// pushes only for its second, submit-only call (see
    /// `FakeHerdr::send_input`'s doc-mirrored behavior and
    /// `SocketHerdr::send_input`'s two real `pane.send_input` calls).
    #[tokio::test]
    async fn terminal_input_with_submit_sends_text_then_enter() {
        let dir = fresh_root("terminal-input-submit");
        enable_terminal(&dir);
        let root = fresh_root("terminal-input-submit-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "input-submit");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .clone()
            .oneshot(terminal_input_req(
                &project.id,
                &started.pane_id,
                "go ahead",
                Some(true),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, Some(&cookie)))
            .await
            .unwrap();
        let body = body_string(screen_resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = json["text"].as_str().unwrap();
        assert!(
            text.ends_with("go ahead\n"),
            "submit=true must send the text then a separate Enter: {text:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-9, truth: "Named keys reach the pane through the keys
    /// path" — pinned against `FakeHerdr::send_keys`'s own echo shape
    /// (non-Enter keys as `<key>` tokens, `enter` as a bare newline) so the
    /// test fails if the handler ever stopped forwarding to `Herdr::send_keys`.
    #[tokio::test]
    async fn terminal_keys_reach_the_pane() {
        let dir = fresh_root("terminal-keys-reach");
        enable_terminal(&dir);
        let root = fresh_root("terminal-keys-reach-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "keys-reach");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .clone()
            .oneshot(terminal_keys_req(
                &project.id,
                &started.pane_id,
                &["down", "enter"],
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, Some(&cookie)))
            .await
            .unwrap();
        let body = body_string(screen_resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = json["text"].as_str().unwrap();
        // agent-terminal-12: `FakeHerdr` echoes non-Enter keys as a literal
        // `<key>` token, which is itself an HTML metacharacter sequence — the
        // ansi translator escapes it like any other screen text, so the
        // assertion checks for the escaped form rather than the raw one.
        assert!(
            text.ends_with("&lt;down&gt;\n"),
            "named keys must reach the pane in order: {text:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-9, truth: "A write aimed at a pane outside the
    /// project's root is refused, even with a valid session" — the D2
    /// containment boundary applied to both write routes, mirroring
    /// `terminal_screen_refuses_a_pane_outside_the_project_root`. The
    /// outside pane's screen is read directly off the fake afterward to
    /// prove the refused write never reached herdr at all, not only that
    /// the HTTP response was a 404.
    #[tokio::test]
    async fn terminal_write_routes_refuse_a_pane_outside_the_project_root() {
        let dir = fresh_root("terminal-write-boundary");
        enable_terminal(&dir);
        let scratch = fresh_root("terminal-write-boundary-scratch");
        let root_a = scratch.join("project-a");
        let outside = scratch.join("outside-a");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let outside_agent = fake
            .agent_start("w1", Some(&outside.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project_a = register(&st, &root_a, "project-a-write");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let input_resp = app
            .clone()
            .oneshot(terminal_input_req(
                &project_a.id,
                &outside_agent.pane_id,
                "should never land",
                Some(true),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(input_resp.status(), StatusCode::NOT_FOUND);
        let input_body = body_string(input_resp).await;
        assert!(input_body.contains("pane not found"), "{input_body}");

        let keys_resp = app
            .oneshot(terminal_keys_req(
                &project_a.id,
                &outside_agent.pane_id,
                &["enter"],
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(keys_resp.status(), StatusCode::NOT_FOUND);
        let keys_body = body_string(keys_resp).await;
        assert!(keys_body.contains("pane not found"), "{keys_body}");

        // Both refusals must never have reached herdr: the outside pane's
        // screen is exactly what `agent_start` seeded it with.
        let read = fake
            .read_pane(&outside_agent.pane_id, herdr::ReadSource::Visible, 0)
            .await
            .unwrap();
        assert_eq!(
            read.text, "❯ ",
            "a refused write must never reach the pane it targeted: {}",
            read.text
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// agent-terminal-9, truth: "Without a terminal session, and with the
    /// terminal switch off, both write endpoints answer byte-identically to
    /// an unrouted path" — the no-session half, mirroring
    /// `terminal_screen_without_a_session_is_byte_identical_to_an_unrouted_path`
    /// for both `/input` and `/keys`.
    #[tokio::test]
    async fn terminal_write_routes_without_a_session_are_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("terminal-write-no-session");
        enable_terminal(&dir);
        let root = fresh_root("terminal-write-no-session-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "write-no-session");
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let unrouted_status = unrouted.status();
            let unrouted_headers = unrouted.headers().clone();
            let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

            let input = app
                .clone()
                .oneshot(terminal_input_req(&project.id, "w1:p1", "hi", Some(true), cookie))
                .await
                .unwrap();
            assert_eq!(input.status(), unrouted_status);
            assert_eq!(input.headers(), &unrouted_headers);
            let input_body = input.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(input_body, unrouted_body);

            let keys = app
                .clone()
                .oneshot(terminal_keys_req(&project.id, "w1:p1", &["enter"], cookie))
                .await
                .unwrap();
            assert_eq!(keys.status(), unrouted_status);
            assert_eq!(keys.headers(), &unrouted_headers);
            let keys_body = keys.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(keys_body, unrouted_body);
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The other half of the same truth: with a valid session but the D7
    /// `terminal.enabled` switch off, both write endpoints still answer
    /// exactly as an unrouted path would — mirroring
    /// `terminal_family_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session`.
    #[tokio::test]
    async fn terminal_write_routes_disabled_are_byte_identical_to_unrouted_even_with_a_valid_session()
    {
        let dir = fresh_root("terminal-write-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("terminal-write-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "write-disabled");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted_status = unrouted.status();
        let unrouted_headers = unrouted.headers().clone();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

        let input = app
            .clone()
            .oneshot(terminal_input_req(&project.id, "w1:p1", "hi", Some(true), Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(input.status(), unrouted_status);
        assert_eq!(input.headers(), &unrouted_headers);
        let input_body = input.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            input_body, unrouted_body,
            "the input endpoint must be unreachable while the switch is off, even with a valid session"
        );

        let keys = app
            .oneshot(terminal_keys_req(&project.id, "w1:p1", &["enter"], Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(keys.status(), unrouted_status);
        assert_eq!(keys.headers(), &unrouted_headers);
        let keys_body = keys.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            keys_body, unrouted_body,
            "the keys endpoint must be unreachable while the switch is off, even with a valid session"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-9, truth: "A wrong-method request to either write
    /// endpoint is byte-identical to an unrouted path" — mounted with
    /// `any(...)` + `MethodGate<Post>`, never `.post(...)`, mirroring
    /// `terminal_screen_wrong_method_is_byte_identical_to_unrouted`.
    #[tokio::test]
    async fn terminal_write_routes_wrong_method_are_byte_identical_to_unrouted() {
        let dir = fresh_root("terminal-write-wrong-method");
        enable_terminal(&dir);
        let root = fresh_root("terminal-write-wrong-method-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "write-wrong-method");
        let app = router(st);

        for path_suffix in ["input", "keys"] {
            let got = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(format!("/p/{}/_terminal/w1:p1/{path_suffix}", project.id))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(got.status(), StatusCode::NOT_FOUND);
            assert_eq!(got.status(), unrouted.status());
            assert_eq!(got.headers(), unrouted.headers());
            let a = got.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// D3/agent-terminal-9: the reply bar and the named-key buttons render on
    /// every pane's card, so `assets/app.js` has a `.term-reply`/`.term-keys`
    /// element to wire up — a view-only assertion that would fail if the
    /// markup were ever dropped independent of the route wiring above.
    #[tokio::test]
    async fn terminal_page_renders_the_reply_bar_and_key_buttons() {
        let dir = fresh_root("terminal-reply-bar-render");
        enable_terminal(&dir);
        let root = fresh_root("terminal-reply-bar-render-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "reply-bar-render");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(terminal_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("class=\"term-reply\" data-pane-id=\"{}\"", started.pane_id)),
            "no reply bar for the pane: {body}"
        );
        assert!(body.contains("class=\"term-reply__send\""), "{body}");
        assert!(body.contains("data-key=\"enter\""), "{body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// agent-terminal-9, truth: "Text sent to a pane never reaches mdview's
    /// logs" — a grep-based proof over this file's own source (the shape
    /// `crates/mdview-core/src/bee.rs`'s `no_web_framework_dependency_declared`
    /// already uses for a source-level guarantee `cargo test` alone can't
    /// otherwise express): no `tracing::*` call anywhere in this file may
    /// reference a typed reply's or key press's body fields. Catches a
    /// regression the instant a future debug log is added, rather than
    /// relying on nobody ever adding one.
    #[test]
    fn typed_text_and_named_keys_never_appear_in_a_tracing_call() {
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/server.rs"))
            .expect("server.rs must be readable from its own crate");
        for (n, line) in src.lines().enumerate() {
            if line.contains("tracing::") {
                assert!(
                    !line.contains("body.text") && !line.contains("body.keys"),
                    "line {}: a typed reply or key press must never reach a tracing/log call: {line}",
                    n + 1
                );
            }
        }
    }

    /// agent-terminal-8, truth: "A request presenting no token, or a wrong
    /// token, cannot obtain a terminal session by any route" — the login
    /// half. Neither an unconfigured token nor a wrong one against a real,
    /// already-generated one ever sets a session cookie; the real token
    /// still works afterward, proving the failed attempts left nothing
    /// corrupted.
    #[tokio::test]
    async fn login_with_a_wrong_or_missing_token_never_mints_a_session() {
        let dir = fresh_root("terminal-login-wrong-token");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        async fn login(app: Router, token: &str) -> Response {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/terminal/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("token={token}")))
                    .unwrap(),
            )
            .await
            .unwrap()
        }

        // No token has ever been generated yet — nothing can verify.
        let resp = login(app.clone(), "whatever").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(resp.headers().get(header::SET_COOKIE).is_none());

        // A real token now exists (first-run rotation); a wrong guess still
        // fails.
        let (real_token, _cookie) = rotate_token(app.clone()).await;
        let resp = login(app.clone(), "definitely-not-it").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(resp.headers().get(header::SET_COOKIE).is_none());

        // The real token still logs in, proving the refusals above changed
        // nothing.
        let resp = login(app, &real_token).await;
        assert!(resp.status().is_redirection());
        assert!(resp.headers().get(header::SET_COOKIE).is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// agent-terminal-8, truth: "Once a token exists, rotation requires the
    /// current session; with no token file yet, first-run rotation is still
    /// possible." Also proves `POST` to the rotation route never itself
    /// returns a usable session cookie, at every stage.
    #[tokio::test]
    async fn rotation_needs_no_session_on_first_run_but_requires_one_after() {
        let dir = fresh_root("terminal-rotation-auth");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        async fn rotate(app: Router, cookie: Option<&str>) -> Response {
            let mut b = Request::builder()
                .method("POST")
                .uri("/settings/terminal/token");
            if let Some(c) = cookie {
                b = b.header(header::COOKIE, c.to_string());
            }
            app.oneshot(b.body(Body::empty()).unwrap()).await.unwrap()
        }

        // First-run: no token file on disk yet, so no session is required.
        let first = rotate(app.clone(), None).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert!(
            first.headers().get(header::SET_COOKIE).is_none(),
            "rotation must never itself set a session cookie"
        );
        let body = body_string(first).await;
        let marker = "it will not be shown again: <code>";
        let start = body.find(marker).unwrap() + marker.len();
        let rest = &body[start..];
        let end = rest.find("</code>").unwrap();
        let token = rest[..end].to_string();

        // A token now exists: rotating again with no session is refused —
        // an unauthenticated visitor can no longer clear the real user's
        // session by re-rotating.
        let second = rotate(app.clone(), None).await;
        assert_eq!(
            second.status(),
            StatusCode::NOT_FOUND,
            "rotating an already-configured token with no session must be refused"
        );

        // Logging in with the current token, then rotating, succeeds — and
        // still never sets a cookie of its own.
        let login_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/terminal/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("token={token}")))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = login_resp
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let third = rotate(app, Some(&cookie)).await;
        assert_eq!(
            third.status(),
            StatusCode::OK,
            "rotating with a live session must succeed"
        );
        assert!(third.headers().get(header::SET_COOKIE).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// agent-terminal-8, truth: "A second device holding the token can sign
    /// in without disturbing the first device's session." Login (unlike
    /// rotation) never clears the session set — only a fresh rotation does
    /// (P5) — so two devices presenting the same still-current token both
    /// end up with their own live, independent sessions.
    #[tokio::test]
    async fn a_second_device_logging_in_does_not_disturb_the_first_devices_session() {
        let dir = fresh_root("terminal-multi-device-login");
        enable_terminal(&dir);
        let root = fresh_root("terminal-multi-device-login-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "multi-device");
        let app = router(st);

        let (token, cookie_a) = rotate_token(app.clone()).await;

        let login_b = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/terminal/login")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!("token={token}")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(login_b.status().is_redirection());
        let cookie_b = login_b
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert_ne!(cookie_a, cookie_b, "each login mints its own distinct session");

        let resp_a = app
            .clone()
            .oneshot(terminal_req(&project.id, Some(&cookie_a)))
            .await
            .unwrap();
        assert_eq!(
            resp_a.status(),
            StatusCode::OK,
            "device A's session must survive device B logging in"
        );

        let resp_b = app
            .oneshot(terminal_req(&project.id, Some(&cookie_b)))
            .await
            .unwrap();
        assert_eq!(resp_b.status(), StatusCode::OK, "device B's own session must work too");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// D6/agent-terminal-8, truth: "The Terminal tab is present on a
    /// registered project that has no .bee directory." `project_home_page`
    /// used to render only for bee projects; a docs-only project redirected
    /// straight to its entry file and never showed the tab strip at all.
    #[tokio::test]
    async fn terminal_tab_is_present_on_a_project_with_no_bee_directory() {
        let dir = fresh_root("terminal-tab-non-bee");
        let root = fresh_root("terminal-tab-non-bee-project");
        write(&root, "README.md", "# Hello\n");

        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "docs-only");
        let resp = get(router(st), &format!("/p/{}/", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("href=\"/p/{}/_terminal\"", project.id)),
            "a non-bee project's home page carries no Terminal tab link: {body}"
        );
        assert!(body.contains(">Terminal<"), "{body}");
        assert!(
            !body.contains("_bee"),
            "a non-bee project's home page must not link to the bee board: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    // ---- agent-terminal-10 (D5): the Unassigned group ----

    /// A GET request to `/_terminal/unassigned`, optionally carrying the
    /// given session cookie value — the group-wide sibling of `terminal_req`.
    fn unassigned_req(cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().uri("/_terminal/unassigned").method("GET");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::empty()).unwrap()
    }

    /// A GET request to `/_terminal/unassigned/{pane_id}/screen`.
    fn unassigned_screen_req(pane_id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/_terminal/unassigned/{pane_id}/screen"))
            .method("GET");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::empty()).unwrap()
    }

    /// A POST request to `/_terminal/unassigned/{pane_id}/input` carrying a
    /// JSON `{ "text": ..., "submit": ... }` body.
    fn unassigned_input_req(pane_id: &str, text: &str, submit: bool, cookie: Option<&str>) -> Request<Body> {
        let body = serde_json::json!({ "text": text, "submit": submit });
        let mut b = Request::builder()
            .uri(format!("/_terminal/unassigned/{pane_id}/input"))
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    /// A POST request to `/_terminal/unassigned/{pane_id}/keys` carrying a
    /// JSON `{ "keys": [...] }` body.
    fn unassigned_keys_req(pane_id: &str, keys: &[&str], cookie: Option<&str>) -> Request<Body> {
        let body = serde_json::json!({ "keys": keys });
        let mut b = Request::builder()
            .uri(format!("/_terminal/unassigned/{pane_id}/keys"))
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    /// Truth: "Without a terminal session, `/_terminal/unassigned` returns
    /// an opaque 404, identical to an unknown route" — the group route's own
    /// instance of the byte-identical-to-unrouted proof every other gated
    /// terminal route already carries.
    #[tokio::test]
    async fn unassigned_route_without_a_session_is_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("unassigned-no-session");
        enable_terminal(&dir);
        let st = build_state_with_dir(&dir);
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let with_no_session = app.clone().oneshot(unassigned_req(cookie)).await.unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(with_no_session.status(), StatusCode::NOT_FOUND);
            assert_eq!(with_no_session.status(), unrouted.status());
            assert_eq!(with_no_session.headers(), unrouted.headers());
            let a = with_no_session.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth (the carry-over obligation, agent-terminal-10's own instance):
    /// a wrong-method request to `/_terminal/unassigned` (mounted via
    /// `any(...)` + `MethodGate<Get>`, never `.get(...)`) is byte-identical
    /// to a path this router never mounts at all.
    #[tokio::test]
    async fn unassigned_route_wrong_method_is_byte_identical_to_unrouted() {
        let dir = fresh_root("unassigned-wrong-method");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_terminal/unassigned")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The must-have this cell shares with agent-terminal-6/8: with the D7
    /// `terminal.enabled` switch off, `/_terminal/unassigned` answers
    /// exactly as an unrouted path would — even with a session a valid
    /// rotation just minted.
    #[tokio::test]
    async fn unassigned_family_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session() {
        let dir = fresh_root("unassigned-family-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let st = build_state_with_dir(&dir);
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let disabled = app.oneshot(unassigned_req(Some(&cookie))).await.unwrap();

        assert_eq!(disabled.status(), StatusCode::NOT_FOUND);
        assert_eq!(disabled.status(), unrouted.status());
        assert_eq!(disabled.headers(), unrouted.headers());
        let a = disabled.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The central D5 partition truth, both directions in one snapshot: a
    /// pane whose cwd sits under a registered project's root is listed on
    /// that project's own `/p/:id/_terminal` and never on
    /// `/_terminal/unassigned`; a pane whose cwd sits under no registered
    /// root is listed on `/_terminal/unassigned` and never on any project's
    /// own page. Registering the project never happens as a side effect of
    /// listing the unassigned pane (D5) — checked by comparing the engine's
    /// project list before and after every request in this test.
    #[tokio::test]
    async fn unassigned_group_and_a_projects_own_terminal_partition_panes_without_overlap() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("unassigned-partition-data");
        enable_terminal(&dir);
        let scratch = fresh_root("unassigned-partition-scratch");
        let project_root = scratch.join("owned-project");
        let stray_root = scratch.join("stray-agent-cwd");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let owned = fake
            .agent_start("w1", Some(&project_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &project_root, "owned-project");
        let engine = st.engine.clone();
        let projects_before = engine.list_projects().unwrap();
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unassigned_resp = app
            .clone()
            .oneshot(unassigned_req(Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(unassigned_resp.status(), StatusCode::OK);
        let unassigned_body = body_string(unassigned_resp).await;
        assert!(
            unassigned_body.contains(&stray.name),
            "the pane under no registered project is missing from the Unassigned group: {unassigned_body}"
        );
        assert!(
            !unassigned_body.contains(&owned.name),
            "a pane already owned by a registered project leaked into the Unassigned group: {unassigned_body}"
        );

        let project_resp = app
            .clone()
            .oneshot(terminal_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        let project_body = body_string(project_resp).await;
        assert!(
            project_body.contains(&owned.name),
            "the project's own pane is missing from its own terminal page: {project_body}"
        );
        assert!(
            !project_body.contains(&stray.name),
            "the unassigned pane leaked into a registered project's own terminal page: {project_body}"
        );

        // D5: listing the stray pane, reading its project's own page, must
        // never register a project from the stray pane's cwd — the engine's
        // registry is exactly what it was before any request in this test.
        let projects_after = engine.list_projects().unwrap();
        assert_eq!(
            projects_before.iter().map(|p| &p.id).collect::<Vec<_>>(),
            projects_after.iter().map(|p| &p.id).collect::<Vec<_>>(),
            "listing an unassigned pane must never add a row to the project registry"
        );
        assert_eq!(
            projects_before.len(),
            1,
            "sanity: exactly one project was registered before any unassigned request"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D5/D4's core resolution, on the home page itself: an unauthenticated
    /// `GET /` must never reveal an unassigned agent's name or cwd, even
    /// though the group's presence marker is visible once the D7 switch is
    /// on. A second request with the switch off proves the page renders
    /// exactly as it did before this feature (no marker, no mention of
    /// "Unassigned" at all).
    #[tokio::test]
    async fn unauthenticated_home_page_shows_unassigned_presence_only_and_leaks_nothing() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-unassigned-presence");
        let scratch = fresh_root("home-unassigned-presence-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        // Switch off (default): the home page must carry no trace of the
        // feature at all — not even the word "Unassigned".
        let mut st_off = build_state_with_dir(&dir);
        st_off.herdr = fake.clone();
        let resp_off = get(router(st_off), "/").await;
        assert_eq!(resp_off.status(), StatusCode::OK);
        let body_off = body_string(resp_off).await;
        assert!(
            !body_off.contains("Unassigned") && !body_off.contains("unassigned"),
            "the home page must render exactly as before when the terminal switch is off: {body_off}"
        );

        // Switch on: the presence marker appears, but the pane's own name
        // and cwd never do — this route takes no session and makes no herdr
        // call, so it structurally cannot leak them.
        enable_terminal(&dir);
        let mut st_on = build_state_with_dir(&dir);
        st_on.herdr = fake;
        let resp_on = get(router(st_on), "/").await;
        assert_eq!(resp_on.status(), StatusCode::OK);
        let body_on = body_string(resp_on).await;
        assert!(
            body_on.contains("Unassigned agents"),
            "the group's presence marker is missing once the switch is on: {body_on}"
        );
        assert!(
            !body_on.contains(&stray.name),
            "an unauthenticated home page leaked an unassigned agent's name: {body_on}"
        );
        assert!(
            !body_on.contains(&stray_root.to_string_lossy().to_string()),
            "an unauthenticated home page leaked an unassigned agent's cwd: {body_on}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D3/agent-terminal-9 parity: the Unassigned group's screen read and
    /// both write paths (free-text input, named keys) reach a pane that is
    /// genuinely unassigned, gated the same way the project-scoped routes
    /// are — and refuse (opaque not-found) a pane that belongs to a
    /// registered project, proving the write paths respect the same
    /// partition the listing page does, not just the read path.
    #[tokio::test]
    async fn unassigned_screen_and_write_routes_reach_only_a_genuinely_unassigned_pane() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("unassigned-write-paths");
        enable_terminal(&dir);
        let scratch = fresh_root("unassigned-write-paths-scratch");
        let project_root = scratch.join("owned");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let owned = fake
            .agent_start("w1", Some(&project_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        register(&st, &project_root, "owned");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        // Screen read reaches the stray pane.
        let screen_resp = app
            .clone()
            .oneshot(unassigned_screen_req(&stray.pane_id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(screen_resp.status(), StatusCode::OK);

        // Screen read refuses the owned pane — it belongs to a project, not
        // to this group.
        let owned_screen_resp = app
            .clone()
            .oneshot(unassigned_screen_req(&owned.pane_id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(owned_screen_resp.status(), StatusCode::NOT_FOUND);

        // Free-text input reaches the stray pane and is readable back.
        let input_resp = app
            .clone()
            .oneshot(unassigned_input_req(&stray.pane_id, "hello stray", true, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(input_resp.status(), StatusCode::OK);
        let input_body = body_string(input_resp).await;
        assert!(input_body.contains("\"ok\":true"), "{input_body}");

        let after_input = app
            .clone()
            .oneshot(unassigned_screen_req(&stray.pane_id, Some(&cookie)))
            .await
            .unwrap();
        let after_input_body = body_string(after_input).await;
        assert!(after_input_body.contains("hello stray"), "{after_input_body}");

        // Named keys reach the stray pane.
        let keys_resp = app
            .clone()
            .oneshot(unassigned_keys_req(&stray.pane_id, &["enter"], Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(keys_resp.status(), StatusCode::OK);

        // Input refuses the owned pane too — the write paths honor the same
        // partition the read path and the listing page do.
        let owned_input_resp = app
            .oneshot(unassigned_input_req(&owned.pane_id, "should never land", true, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(owned_input_resp.status(), StatusCode::NOT_FOUND);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    // ---- agent-terminal-11: the Unassigned group's own guard tests ----
    //
    // A semantic review found every request the suite made to the three
    // unassigned routes (screen, input, keys) carried a valid cookie --
    // removing `AuthSession` from any of the three failed nothing. These
    // pin the same three truths (no-session, wrong-method, switch-off)
    // their project-scoped equivalents already carry, proven the same
    // byte-identical-to-unrouted way.

    /// Truth: without a terminal session,
    /// `/_terminal/unassigned/{pane}/screen` answers byte-identically to an
    /// unrouted path -- mirroring
    /// `terminal_screen_without_a_session_is_byte_identical_to_an_unrouted_path`.
    /// Verified by temporarily deleting `_session: AuthSession` from
    /// `unassigned_terminal_screen`: with no other guard in front of it, the
    /// handler falls through to `verify_pane_is_unassigned`'s ordinary
    /// `not_found("pane not found")` -- an HTML body, distinct from this
    /// opaque empty-body 404 -- so this assertion goes red exactly as its
    /// project-scoped sibling's does.
    #[tokio::test]
    async fn unassigned_screen_without_a_session_is_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("unassigned-screen-no-session");
        enable_terminal(&dir);
        let st = build_state_with_dir(&dir);
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let with_no_session = app
                .clone()
                .oneshot(unassigned_screen_req("w1:p1", cookie))
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(with_no_session.status(), StatusCode::NOT_FOUND);
            assert_eq!(with_no_session.status(), unrouted.status());
            assert_eq!(with_no_session.headers(), unrouted.headers());
            let a = with_no_session.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth: without a terminal session, both unassigned write routes
    /// (`/input`, `/keys`) answer byte-identically to an unrouted path --
    /// mirroring
    /// `terminal_write_routes_without_a_session_are_byte_identical_to_an_unrouted_path`.
    /// Verified the same way as the screen route above, for both
    /// `unassigned_terminal_input` and `unassigned_terminal_keys`.
    #[tokio::test]
    async fn unassigned_write_routes_without_a_session_are_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("unassigned-write-no-session");
        enable_terminal(&dir);
        let st = build_state_with_dir(&dir);
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let unrouted_status = unrouted.status();
            let unrouted_headers = unrouted.headers().clone();
            let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

            let input = app
                .clone()
                .oneshot(unassigned_input_req("w1:p1", "hi", true, cookie))
                .await
                .unwrap();
            assert_eq!(input.status(), unrouted_status);
            assert_eq!(input.headers(), &unrouted_headers);
            let input_body = input.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(input_body, unrouted_body);

            let keys = app
                .clone()
                .oneshot(unassigned_keys_req("w1:p1", &["enter"], cookie))
                .await
                .unwrap();
            assert_eq!(keys.status(), unrouted_status);
            assert_eq!(keys.headers(), &unrouted_headers);
            let keys_body = keys.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(keys_body, unrouted_body);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth: a wrong-method request to `/_terminal/unassigned/{pane}/screen`
    /// (mounted via `any(...)` + `MethodGate<Get>`, never `.get(...)`) is
    /// byte-identical to an unrouted path -- mirroring
    /// `terminal_screen_wrong_method_is_byte_identical_to_unrouted`.
    #[tokio::test]
    async fn unassigned_screen_wrong_method_is_byte_identical_to_unrouted() {
        let dir = fresh_root("unassigned-screen-wrong-method");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_terminal/unassigned/w1:p1/screen")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth: a wrong-method request to either unassigned write route is
    /// byte-identical to an unrouted path -- mirroring
    /// `terminal_write_routes_wrong_method_are_byte_identical_to_unrouted`.
    #[tokio::test]
    async fn unassigned_write_routes_wrong_method_are_byte_identical_to_unrouted() {
        let dir = fresh_root("unassigned-write-wrong-method");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        for path_suffix in ["input", "keys"] {
            let got = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(format!("/_terminal/unassigned/w1:p1/{path_suffix}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(got.status(), StatusCode::NOT_FOUND);
            assert_eq!(got.status(), unrouted.status());
            assert_eq!(got.headers(), unrouted.headers());
            let a = got.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth: with a valid session but the D7 `terminal.enabled` switch off,
    /// `/_terminal/unassigned/{pane}/screen` still answers exactly as an
    /// unrouted path would -- mirroring
    /// `terminal_family_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session`.
    #[tokio::test]
    async fn unassigned_screen_disabled_is_byte_identical_to_unrouted_even_with_a_valid_session() {
        let dir = fresh_root("unassigned-screen-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let st = build_state_with_dir(&dir);
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted_status = unrouted.status();
        let unrouted_headers = unrouted.headers().clone();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

        let screen = app
            .oneshot(unassigned_screen_req("w1:p1", Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(screen.status(), unrouted_status);
        assert_eq!(screen.headers(), &unrouted_headers);
        let screen_body = screen.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            screen_body, unrouted_body,
            "the unassigned screen endpoint must be unreachable while the switch is off, even with a valid session"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth: with a valid session but the switch off, both unassigned write
    /// routes still answer exactly as an unrouted path would -- mirroring
    /// `terminal_write_routes_disabled_are_byte_identical_to_unrouted_even_with_a_valid_session`.
    #[tokio::test]
    async fn unassigned_write_routes_disabled_are_byte_identical_to_unrouted_even_with_a_valid_session()
    {
        let dir = fresh_root("unassigned-write-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let st = build_state_with_dir(&dir);
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted_status = unrouted.status();
        let unrouted_headers = unrouted.headers().clone();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

        let input = app
            .clone()
            .oneshot(unassigned_input_req("w1:p1", "hi", true, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(input.status(), unrouted_status);
        assert_eq!(input.headers(), &unrouted_headers);
        let input_body = input.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            input_body, unrouted_body,
            "the unassigned input endpoint must be unreachable while the switch is off, even with a valid session"
        );

        let keys = app
            .oneshot(unassigned_keys_req("w1:p1", &["enter"], Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(keys.status(), unrouted_status);
        assert_eq!(keys.headers(), &unrouted_headers);
        let keys_body = keys.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            keys_body, unrouted_body,
            "the unassigned keys endpoint must be unreachable while the switch is off, even with a valid session"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth (the registry-failure must-have): a project whose own boundary
    /// cannot be constructed never widens the set of panes the Unassigned
    /// group exposes -- the same fail-closed code path this cell adds for a
    /// registry read failure (`unassigned_panes` returning `Vec::new()`
    /// rather than letting the project's own panes fall through as
    /// "unassigned"). Forced deterministically by registering a project
    /// whose root sits under `/etc`, on `paths_boundary::hard_deny_list`,
    /// which `Boundary::new` refuses to construct on every platform this
    /// suite runs on -- no locking or timing involved, unlike trying to
    /// force a genuine SQLite I/O error on an already-open, long-lived
    /// connection from a second connection in the same process, which this
    /// cell found to be unreliable in this workspace (evidence: a dropped
    /// table is visible to a *fresh* connection immediately but not to the
    /// engine's own long-lived one, most likely because `crates/mdview`'s
    /// dev-only `rusqlite` and `mdview-core`'s `rusqlite` link two separate
    /// copies of the bundled SQLite that don't share WAL/locking state --
    /// `mdview-core` has no public seam to inject a failing store, so a
    /// dependable registry-Err test is out of this cell's file scope).
    #[tokio::test]
    async fn unassigned_group_fails_closed_when_a_projects_boundary_is_unconstructable() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("unassigned-boundary-unconstructable");
        enable_terminal(&dir);
        let scratch = fresh_root("unassigned-boundary-unconstructable-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();
        let denied_root = PathBuf::from("/etc/mdview-test-fixture-nonexistent");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        // A pane whose cwd sits under the hard-deny-listed project's own
        // root -- this is the pane that must NOT leak into Unassigned.
        let denied_project_pane = fake
            .agent_start("w1", Some(&denied_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &denied_root, "denied-root-project");
        assert_eq!(
            project.root_path, denied_root,
            "sanity: canonicalize falls back to the literal path when it doesn't exist"
        );
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app.oneshot(unassigned_req(Some(&cookie))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            !body.contains(&denied_project_pane.name) && !body.contains(&stray.name),
            "a project whose boundary cannot be constructed must fail the whole group closed to \
             zero panes, not leak its own panes into Unassigned: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Truth (the carried-over method-mismatch-oracle obligation, isolated):
    /// at least one wrong-method test must fail if `MethodGate` were removed
    /// while a valid session and an enabled terminal are already in place.
    /// Every other wrong-method test above runs with no session and the
    /// switch off, so it would still pass unchanged even with `MethodGate`
    /// deleted entirely -- `AuthSession`'s own rejection, or the
    /// disabled-switch check, would answer the opaque 404 first, without
    /// `MethodGate` ever mattering. Verified by temporarily deleting
    /// `_method: MethodGate<Get>` from `terminal_page`: the POST then
    /// reaches the handler body and (against the enabled switch, the
    /// registered project, and the default available `FakeHerdr`) renders
    /// `200 OK` with an empty pane list -- nothing like the unrouted path's
    /// bare 404 -- so this assertion goes red.
    #[tokio::test]
    async fn terminal_route_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal()
    {
        let dir = fresh_root("terminal-wrong-method-isolated");
        enable_terminal(&dir);
        let root = fresh_root("terminal-wrong-method-isolated-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "wrong-method-isolated");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let posted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/p/{}/_terminal", project.id))
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(posted.status(), StatusCode::NOT_FOUND);
        assert_eq!(posted.status(), unrouted.status());
        assert_eq!(posted.headers(), unrouted.headers());
        let a = posted.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: the token-rotation route, now mounted with `any(...)` +
    /// `MethodGate<Post>` instead of the `.post(...)` this cell replaces, is
    /// unreachable by any method but `POST` -- mirroring
    /// `api_terminal_config_wrong_method_is_byte_identical_to_unrouted`.
    /// Verified by temporarily reverting the route table to
    /// `.route("/settings/terminal/token", post(rotate_terminal_token))`: a
    /// `GET` then answers `405 Allow: POST` instead of matching the unrouted
    /// 404, and this assertion goes red.
    #[tokio::test]
    async fn rotate_terminal_token_wrong_method_is_byte_identical_to_unrouted() {
        let dir = fresh_root("terminal-token-wrong-method");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let got = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/settings/terminal/token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(got.status(), StatusCode::NOT_FOUND);
        assert_eq!(got.status(), unrouted.status());
        assert_eq!(got.headers(), unrouted.headers());
        let a = got.into_body().collect().await.unwrap().to_bytes();
        let b = unrouted.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Truth (must-have): a single `/keys` request cannot queue an
    /// unbounded number of key presses. `MAX_KEYS_PER_REQUEST` refuses a
    /// request over the bound with `400`, before it ever reaches herdr --
    /// proven against a real pane, so a bug that let the oversized list
    /// through would show up as the pane's screen actually changing.
    #[tokio::test]
    async fn terminal_keys_request_exceeding_the_bound_is_refused_without_reaching_herdr() {
        let dir = fresh_root("terminal-keys-bound");
        enable_terminal(&dir);
        let root = fresh_root("terminal-keys-bound-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project = register(&st, &root, "keys-bound");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let too_many: Vec<&str> = std::iter::repeat("enter")
            .take(MAX_KEYS_PER_REQUEST + 1)
            .collect();
        let resp = app
            .oneshot(terminal_keys_req(
                &project.id,
                &started.pane_id,
                &too_many,
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // The refusal must never have reached herdr: the pane's screen is
        // exactly what `agent_start` seeded it with.
        let read = fake
            .read_pane(&started.pane_id, herdr::ReadSource::Visible, 0)
            .await
            .unwrap();
        assert_eq!(
            read.text, "❯ ",
            "an oversized keys request must never reach the pane it targeted: {}",
            read.text
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The Unassigned group's own instance of the same bound.
    #[tokio::test]
    async fn unassigned_keys_request_exceeding_the_bound_is_refused_without_reaching_herdr() {
        let dir = fresh_root("unassigned-keys-bound");
        enable_terminal(&dir);
        let scratch = fresh_root("unassigned-keys-bound-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let too_many: Vec<&str> = std::iter::repeat("enter")
            .take(MAX_KEYS_PER_REQUEST + 1)
            .collect();
        let resp = app
            .oneshot(unassigned_keys_req(&stray.pane_id, &too_many, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let read = fake
            .read_pane(&stray.pane_id, herdr::ReadSource::Visible, 0)
            .await
            .unwrap();
        assert_eq!(
            read.text, "❯ ",
            "an oversized keys request must never reach the pane it targeted: {}",
            read.text
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    // --- agent-terminal-13: the create routes -------------------------------

    /// A minimal `Herdr` double for the create routes' resolution tests: its
    /// `snapshot()` always answers the fixed snapshot it was built with, and
    /// `tab_create`/`agent_start` record what they were actually called with
    /// and answer a synthesized success — enough to prove what the routes
    /// decided to send, without reimplementing `FakeHerdr`'s own pane
    /// bookkeeping (`FakeHerdr`'s seeded workspaces never move their own
    /// anchor: `agent_start`/`tab_create` only add sibling panes to the
    /// existing tab). Every other `Herdr` method is unreachable — the create
    /// routes never call them. Mirrors herdr-go's own `RecordingHerdr`
    /// (`herdr-go/src/web/create.rs`).
    struct RecordingHerdr {
        snap: herdr::Snapshot,
        tab_calls: std::sync::Mutex<Vec<(String, Option<String>)>>,
        agent_calls: std::sync::Mutex<Vec<(String, Option<String>, Vec<String>)>>,
        /// When set, `tab_create`/`agent_start` return this error instead of
        /// a synthesized success — taken (moved out) on the first call, since
        /// `herdr::HerdrError` carries no `Clone` impl. This is the one
        /// combination `FakeHerdr::set_available(false)` can't produce: that
        /// fails `snapshot()` itself (`terminal_create_routes_herdr_down_is_502`),
        /// so `create_error_response` is never reached at all — here
        /// `snapshot()` still succeeds and resolves a destination, and only
        /// the placement call itself fails.
        fail: std::sync::Mutex<Option<herdr::HerdrError>>,
    }

    impl RecordingHerdr {
        fn new(snap: herdr::Snapshot) -> Self {
            RecordingHerdr {
                snap,
                tab_calls: std::sync::Mutex::new(Vec::new()),
                agent_calls: std::sync::Mutex::new(Vec::new()),
                fail: std::sync::Mutex::new(None),
            }
        }

        /// Same as `new`, but `tab_create`/`agent_start` fail with `err`
        /// instead of succeeding — see the `fail` field's doc.
        fn new_failing(snap: herdr::Snapshot, err: herdr::HerdrError) -> Self {
            RecordingHerdr {
                snap,
                tab_calls: std::sync::Mutex::new(Vec::new()),
                agent_calls: std::sync::Mutex::new(Vec::new()),
                fail: std::sync::Mutex::new(Some(err)),
            }
        }
    }

    #[async_trait::async_trait]
    impl herdr::Herdr for RecordingHerdr {
        async fn snapshot(&self) -> herdr::Result<herdr::Snapshot> {
            Ok(self.snap.clone())
        }
        async fn ping(&self) -> herdr::Result<herdr::ProtocolInfo> {
            unreachable!("create routes never ping")
        }
        async fn read_pane(
            &self,
            _pane_id: &str,
            _source: herdr::ReadSource,
            _lines: usize,
        ) -> herdr::Result<herdr::ScreenRead> {
            unreachable!("create routes never read")
        }
        async fn send_input(&self, _pane_id: &str, _text: &str, _submit: bool) -> herdr::Result<()> {
            unreachable!("create routes never send input")
        }
        async fn send_text(&self, _pane_id: &str, _bytes: &str) -> herdr::Result<()> {
            unreachable!("create routes never send text")
        }
        async fn send_keys(&self, _pane_id: &str, _keys: &[String]) -> herdr::Result<()> {
            unreachable!("create routes never send keys")
        }
        async fn tab_create(
            &self,
            workspace_id: &str,
            cwd: Option<&str>,
        ) -> herdr::Result<herdr::TabCreated> {
            self.tab_calls
                .lock()
                .unwrap()
                .push((workspace_id.to_string(), cwd.map(str::to_string)));
            if let Some(err) = self.fail.lock().unwrap().take() {
                return Err(err);
            }
            Ok(herdr::TabCreated {
                tab_id: format!("{workspace_id}:created-tab"),
                pane_id: format!("{workspace_id}:created-pane"),
            })
        }
        async fn agent_start(
            &self,
            workspace_id: &str,
            cwd: Option<&str>,
            argv: &[String],
        ) -> herdr::Result<herdr::AgentStarted> {
            self.agent_calls
                .lock()
                .unwrap()
                .push((workspace_id.to_string(), cwd.map(str::to_string), argv.to_vec()));
            if let Some(err) = self.fail.lock().unwrap().take() {
                return Err(err);
            }
            Ok(herdr::AgentStarted {
                tab_id: format!("{workspace_id}:created-agent-tab"),
                pane_id: format!("{workspace_id}:created-agent-pane"),
                name: "recorded-agent".into(),
            })
        }
    }

    /// A `herdr::Snapshot` with exactly one workspace ("w") whose D2 anchor
    /// resolves at `path` — a workspace, its own active tab, a layout entry
    /// naming a focused pane, and that pane's own `cwd`/`foreground_cwd`,
    /// reproducing the real join `Snapshot::anchor_for_workspace` performs.
    fn resolvable_snapshot(path: &Path) -> herdr::Snapshot {
        let p = path.to_string_lossy().into_owned();
        herdr::Snapshot {
            workspaces: vec![herdr::wire::Workspace {
                workspace_id: "w".into(),
                label: "w".into(),
                agent_status: herdr::AgentStatus::Idle,
                active_tab_id: Some("w:t".into()),
            }],
            tabs: vec![herdr::wire::Tab {
                tab_id: "w:t".into(),
                label: "main".into(),
            }],
            layouts: vec![herdr::wire::PaneLayout {
                workspace_id: "w".into(),
                tab_id: "w:t".into(),
                focused_pane_id: Some("w:p".into()),
            }],
            panes: vec![herdr::wire::Pane {
                pane_id: "w:p".into(),
                workspace_id: "w".into(),
                tab_id: "w:t".into(),
                cwd: Some(p.clone()),
                foreground_cwd: Some(p),
            }],
            ..herdr::Snapshot::default()
        }
    }

    /// Writes one D8 preset into `dir/config.toml`, preserving whatever is
    /// already there (mirrors `enable_terminal`'s load-mutate-save shape).
    fn configure_preset(dir: &Path, label: &str, argv: &[&str]) {
        let mut cfg = Config::load_from(&dir.join("config.toml"));
        cfg.terminal.agent_presets.push(mdview_core::config::AgentPreset {
            label: label.to_string(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
        });
        cfg.save_to(&dir.join("config.toml")).unwrap();
    }

    fn create_pane_req(id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal/create/pane"))
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::from("{}")).unwrap()
    }

    fn create_agent_req(id: &str, body: &serde_json::Value, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal/create/agent"))
            .method("POST")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    /// Truth: without a terminal session both create routes answer exactly
    /// as an unrouted path would — mirroring
    /// `terminal_write_routes_without_a_session_are_byte_identical_to_an_unrouted_path`.
    #[tokio::test]
    async fn terminal_create_routes_without_a_session_are_byte_identical_to_an_unrouted_path() {
        let dir = fresh_root("terminal-create-no-session");
        enable_terminal(&dir);
        let root = fresh_root("terminal-create-no-session-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "create-no-session");
        let app = router(st);

        for cookie in [None, Some("mdview_terminal_session=not-a-real-session")] {
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let unrouted_status = unrouted.status();
            let unrouted_headers = unrouted.headers().clone();
            let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

            let pane = app
                .clone()
                .oneshot(create_pane_req(&project.id, cookie))
                .await
                .unwrap();
            assert_eq!(pane.status(), unrouted_status);
            assert_eq!(pane.headers(), &unrouted_headers);
            let pane_body = pane.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(pane_body, unrouted_body);

            let agent = app
                .clone()
                .oneshot(create_agent_req(
                    &project.id,
                    &serde_json::json!({ "preset": "Claude" }),
                    cookie,
                ))
                .await
                .unwrap();
            assert_eq!(agent.status(), unrouted_status);
            assert_eq!(agent.headers(), &unrouted_headers);
            let agent_body = agent.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(agent_body, unrouted_body);
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: a wrong-method request to either create route is
    /// byte-identical to an unrouted path — mounted with `any(...)` +
    /// `MethodGate<Post>`, never `.post(...)`.
    #[tokio::test]
    async fn terminal_create_routes_wrong_method_are_byte_identical_to_unrouted() {
        let dir = fresh_root("terminal-create-wrong-method");
        enable_terminal(&dir);
        let root = fresh_root("terminal-create-wrong-method-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "create-wrong-method");
        let app = router(st);

        for path_suffix in ["create/pane", "create/agent"] {
            let got = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri(format!("/p/{}/_terminal/{path_suffix}", project.id))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let unrouted = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("GET")
                        .uri("/this-path-was-never-routed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(got.status(), StatusCode::NOT_FOUND);
            assert_eq!(got.status(), unrouted.status());
            assert_eq!(got.headers(), unrouted.headers());
            let a = got.into_body().collect().await.unwrap().to_bytes();
            let b = unrouted.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(a, b);
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth (the carried-over method-mismatch-oracle obligation, isolated —
    /// mirrors
    /// `terminal_route_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal`):
    /// `terminal_create_routes_wrong_method_are_byte_identical_to_unrouted`
    /// above sends no session cookie at all, so it would still pass
    /// unchanged even with `MethodGate` deleted from either creation
    /// handler entirely — `AuthSession`'s own rejection refuses the request
    /// first, without `MethodGate` ever mattering. Verified by temporarily
    /// deleting `_method: MethodGate<Post>,` from `terminal_create_pane` and
    /// (separately) from `terminal_create_agent`: a GET carrying the same
    /// JSON body a real POST would send then reaches the handler body
    /// (against a valid session, the enabled switch, and the registered
    /// project) and renders `409` — the default `FakeHerdr`'s fixture
    /// anchors resolve under no real project root, so
    /// `destination_unresolved_response` fires — nothing like the unrouted
    /// path's bare 404, so this assertion goes red. Verified both ways
    /// (each guard deleted, tested, then restored) before capping.
    #[tokio::test]
    async fn terminal_create_routes_wrong_method_isolates_method_gate_with_a_valid_session_and_enabled_terminal()
    {
        let dir = fresh_root("terminal-create-wrong-method-isolated");
        enable_terminal(&dir);
        let root = fresh_root("terminal-create-wrong-method-isolated-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "create-wrong-method-isolated");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted_status = unrouted.status();
        let unrouted_headers = unrouted.headers().clone();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

        let pane = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/p/{}/_terminal/create/pane", project.id))
                    .header(header::COOKIE, cookie.clone())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pane.status(), unrouted_status);
        assert_eq!(pane.headers(), &unrouted_headers);
        let pane_body = pane.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(pane_body, unrouted_body);

        let agent = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/p/{}/_terminal/create/agent", project.id))
                    .header(header::COOKIE, cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::json!({ "preset": "Claude" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(agent.status(), unrouted_status);
        assert_eq!(agent.headers(), &unrouted_headers);
        let agent_body = agent.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(agent_body, unrouted_body);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: with a valid session but the D7 switch off, both create
    /// routes still answer exactly as an unrouted path would.
    #[tokio::test]
    async fn terminal_create_routes_disabled_are_byte_identical_to_unrouted_even_with_a_valid_session()
    {
        let dir = fresh_root("terminal-create-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("terminal-create-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "create-disabled");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let unrouted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/this-path-was-never-routed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let unrouted_status = unrouted.status();
        let unrouted_headers = unrouted.headers().clone();
        let unrouted_body = unrouted.into_body().collect().await.unwrap().to_bytes();

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(pane.status(), unrouted_status);
        assert_eq!(pane.headers(), &unrouted_headers);
        let pane_body = pane.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            pane_body, unrouted_body,
            "the pane-create endpoint must be unreachable while the switch is off, even with a valid session"
        );

        let agent = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(agent.status(), unrouted_status);
        assert_eq!(agent.headers(), &unrouted_headers);
        let agent_body = agent.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            agent_body, unrouted_body,
            "the agent-create endpoint must be unreachable while the switch is off, even with a valid session"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: an unconfigured preset label is refused with 400 and never
    /// reaches herdr — proven both when zero presets are configured at all
    /// (the must-have's literal wording: "with no presets configured, the
    /// preset route starts nothing") and when other presets exist but the
    /// requested label is not among them.
    #[tokio::test]
    async fn terminal_create_agent_unknown_preset_is_400_and_reaches_no_port() {
        let dir = fresh_root("terminal-create-unknown-preset");
        enable_terminal(&dir);
        let root = fresh_root("terminal-create-unknown-preset-project");
        std::fs::create_dir_all(&root).unwrap();

        let herdr = std::sync::Arc::new(RecordingHerdr::new(resolvable_snapshot(&root)));
        let mut st = build_state_with_dir(&dir);
        st.herdr = herdr.clone();
        let project = register(&st, &root, "create-unknown-preset");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        // No presets configured at all.
        let resp = app
            .clone()
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // One preset configured, but a different label requested.
        configure_preset(&dir, "Claude", &["claude"]);
        let resp = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Codex" }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        assert!(
            herdr.agent_calls.lock().unwrap().is_empty(),
            "an unconfigured preset label must never reach agent_start"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: a project with no herdr workspace whose anchor resolves under
    /// its own root refuses creation with 409 — never a silent fallback to
    /// any other directory. `FakeHerdr`'s default seed anchors point at
    /// fixture paths that do not exist on this filesystem at all, so none
    /// of them can ever validate against any real project root.
    #[tokio::test]
    async fn terminal_create_routes_destination_unresolved_is_409() {
        let dir = fresh_root("terminal-create-unresolved");
        enable_terminal(&dir);
        configure_preset(&dir, "Claude", &["claude"]);
        let root = fresh_root("terminal-create-unresolved-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "create-unresolved");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(pane.status(), StatusCode::CONFLICT);

        let agent = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(agent.status(), StatusCode::CONFLICT);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: a workspace that exists and resolves, but *outside* this
    /// project's own root, is never picked as a destination — 409, not a
    /// process started under a project the session is not looking at.
    #[tokio::test]
    async fn terminal_create_routes_refuse_a_destination_outside_the_project_root() {
        let dir = fresh_root("terminal-create-boundary");
        enable_terminal(&dir);
        configure_preset(&dir, "Claude", &["claude"]);
        let scratch = fresh_root("terminal-create-boundary-scratch");
        let root_a = scratch.join("project-a");
        let outside = scratch.join("outside-a");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let herdr = std::sync::Arc::new(RecordingHerdr::new(resolvable_snapshot(&outside)));
        let mut st = build_state_with_dir(&dir);
        st.herdr = herdr.clone();
        let project_a = register(&st, &root_a, "create-boundary-a");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project_a.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(pane.status(), StatusCode::CONFLICT);

        let agent = app
            .oneshot(create_agent_req(
                &project_a.id,
                &serde_json::json!({ "preset": "Claude" }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(agent.status(), StatusCode::CONFLICT);

        assert!(
            herdr.tab_calls.lock().unwrap().is_empty(),
            "a destination outside the project root must never reach tab_create"
        );
        assert!(
            herdr.agent_calls.lock().unwrap().is_empty(),
            "a destination outside the project root must never reach agent_start"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Truth: a herdr port failure maps to 502 on both routes.
    #[tokio::test]
    async fn terminal_create_routes_herdr_down_is_502() {
        let dir = fresh_root("terminal-create-herdr-down");
        enable_terminal(&dir);
        configure_preset(&dir, "Claude", &["claude"]);
        let root = fresh_root("terminal-create-herdr-down-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        fake.set_available(false);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "create-herdr-down");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(pane.status(), StatusCode::BAD_GATEWAY);

        let agent = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(agent.status(), StatusCode::BAD_GATEWAY);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: `create_error_response` — reached by no other test, since
    /// `terminal_create_routes_herdr_down_is_502` fails earlier, at the
    /// `snapshot()` call, and never reaches it — maps a herdr placement
    /// failure (`WorkspaceNotFound`, or a `Remote` error carrying one of the
    /// two placement codes) to `409`, and every other herdr error to `502`.
    /// `RecordingHerdr::new_failing` lets `snapshot()` still succeed (so a
    /// destination resolves and the route actually calls herdr) while the
    /// placement call itself fails — the one combination
    /// `terminal_create_routes_herdr_down_is_502`'s herdr-down double can't
    /// produce. Both branches are exercised, on the two different create
    /// routes, so replacing this function's body with any single status —
    /// 409 always, or 502 always — makes one of the two assertions below go
    /// red.
    #[tokio::test]
    async fn terminal_create_routes_map_a_workspace_conflict_to_409_and_any_other_herdr_error_to_502()
    {
        let dir = fresh_root("terminal-create-error-mapping");
        enable_terminal(&dir);
        let root = fresh_root("terminal-create-error-mapping-project");
        std::fs::create_dir_all(&root).unwrap();

        // `tab_create` itself fails with a workspace-placement error (the
        // destination resolved fine; herdr refused the actual creation) —
        // must map to 409, never the generic 502.
        let conflict_herdr = std::sync::Arc::new(RecordingHerdr::new_failing(
            resolvable_snapshot(&root),
            herdr::HerdrError::WorkspaceNotFound {
                workspace_id: "w".into(),
                message: "placement is gone".into(),
            },
        ));
        let mut st = build_state_with_dir(&dir);
        st.herdr = conflict_herdr;
        let project = register(&st, &root, "create-error-mapping");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let pane = app
            .oneshot(create_pane_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            pane.status(),
            StatusCode::CONFLICT,
            "a herdr WorkspaceNotFound failure must map to 409: {}",
            body_string(pane).await
        );

        // A different project, so `agent_start` gets its own fresh double —
        // an ordinary request failure, which must map to 502, never 409.
        let dir2 = fresh_root("terminal-create-error-mapping-2");
        enable_terminal(&dir2);
        configure_preset(&dir2, "Claude", &["claude"]);
        let root2 = fresh_root("terminal-create-error-mapping-project-2");
        std::fs::create_dir_all(&root2).unwrap();
        let other_herdr = std::sync::Arc::new(RecordingHerdr::new_failing(
            resolvable_snapshot(&root2),
            herdr::HerdrError::Request("boom".into()),
        ));
        let mut st2 = build_state_with_dir(&dir2);
        st2.herdr = other_herdr;
        let project2 = register(&st2, &root2, "create-error-mapping-2");
        let app2 = router(st2);
        let (_token2, cookie2) = rotate_token(app2.clone()).await;

        let agent = app2
            .oneshot(create_agent_req(
                &project2.id,
                &serde_json::json!({ "preset": "Claude" }),
                Some(&cookie2),
            ))
            .await
            .unwrap();
        assert_eq!(
            agent.status(),
            StatusCode::BAD_GATEWAY,
            "a generic herdr failure must map to 502, never 409: {}",
            body_string(agent).await
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&dir2).ok();
        std::fs::remove_dir_all(&root2).ok();
    }

    /// Truth: a resolved destination creates a plain shell and returns its
    /// ids, seeding herdr's own call with this project's resolved root as
    /// `cwd` (never `None`).
    #[tokio::test]
    async fn terminal_create_pane_creates_shell_and_returns_ids() {
        let dir = fresh_root("terminal-create-pane-ok");
        enable_terminal(&dir);
        let root = fresh_root("terminal-create-pane-ok-project");
        std::fs::create_dir_all(&root).unwrap();

        let herdr = std::sync::Arc::new(RecordingHerdr::new(resolvable_snapshot(&root)));
        let mut st = build_state_with_dir(&dir);
        st.herdr = herdr.clone();
        let project = register(&st, &root, "create-pane-ok");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(create_pane_req(&project.id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(json["tab_id"].as_str().unwrap().starts_with("w:"), "{body}");
        assert!(json["pane_id"].as_str().unwrap().starts_with("w:"), "{body}");

        let calls = herdr.tab_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "w");
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        assert_eq!(calls[0].1.as_deref(), Some(canonical_root.to_str().unwrap()));

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Truth: a resolved destination starts an agent using the configured
    /// preset's own argv, returning ids and the herdr-assigned name.
    #[tokio::test]
    async fn terminal_create_agent_creates_and_returns_ids_and_name_using_configured_argv() {
        let dir = fresh_root("terminal-create-agent-ok");
        enable_terminal(&dir);
        configure_preset(&dir, "Claude", &["claude"]);
        let root = fresh_root("terminal-create-agent-ok-project");
        std::fs::create_dir_all(&root).unwrap();

        let herdr = std::sync::Arc::new(RecordingHerdr::new(resolvable_snapshot(&root)));
        let mut st = build_state_with_dir(&dir);
        st.herdr = herdr.clone();
        let project = register(&st, &root, "create-agent-ok");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(json["tab_id"].is_string(), "{body}");
        assert!(json["pane_id"].is_string(), "{body}");
        assert_eq!(json["name"], serde_json::json!("recorded-agent"));

        let calls = herdr.agent_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "w");
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        assert_eq!(calls[0].1.as_deref(), Some(canonical_root.to_str().unwrap()));
        assert_eq!(calls[0].2, vec!["claude".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The must-have this test exists to prove: "no argv, env or cwd
    /// supplied by the request can influence what is started, because no
    /// such field is deserialized." An extra `argv` key riding alongside a
    /// valid `preset` in the JSON body is silently dropped by serde
    /// (`CreateAgentBody` declares only `preset`) — the agent that actually
    /// starts carries the *configured* preset's argv, never the body's.
    #[tokio::test]
    async fn terminal_create_agent_extra_argv_key_in_body_is_ignored() {
        let dir = fresh_root("terminal-create-agent-extra-argv");
        enable_terminal(&dir);
        configure_preset(&dir, "Claude", &["claude"]);
        let root = fresh_root("terminal-create-agent-extra-argv-project");
        std::fs::create_dir_all(&root).unwrap();

        let herdr = std::sync::Arc::new(RecordingHerdr::new(resolvable_snapshot(&root)));
        let mut st = build_state_with_dir(&dir);
        st.herdr = herdr.clone();
        let project = register(&st, &root, "create-agent-extra-argv");
        let app = router(st);
        let (_token, cookie) = rotate_token(app.clone()).await;

        let resp = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({
                    "preset": "Claude",
                    "argv": ["rm", "-rf", "/"],
                    "env": { "EVIL": "1" },
                    "cwd": "/etc",
                }),
                Some(&cookie),
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "an extra argv/env/cwd key must be ignored, not rejected: {}",
            body_string(resp).await
        );

        let calls = herdr.agent_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].2,
            vec!["claude".to_string()],
            "the started argv must be the configured preset's own, never anything from the request body"
        );
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        assert_eq!(
            calls[0].1.as_deref(),
            Some(canonical_root.to_str().unwrap()),
            "the request's own bogus \"cwd\" must never reach herdr"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }
}
