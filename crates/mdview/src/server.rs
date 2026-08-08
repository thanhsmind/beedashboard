//! Axum daemon: routes, live-reload WebSocket, filesystem watcher.

use crate::herdr::{self, Herdr};
use crate::runtime::{self, DaemonInfo};
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
use std::time::Duration;
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
    /// instance per `AppState`, shared via its own internal `Arc`-free
    /// interior mutability across every clone.
    pub terminal_background: Arc<crate::TerminalBackground>,
    /// The D7/D9 notification outbox (agent-terminal-17): opened once at
    /// startup — in-memory for every route test, so no test ever touches
    /// the real `~/.mdview/notify.sqlite` — and handed to
    /// `terminal_background` on every reconcile rather than reopened per
    /// toggle.
    pub notify_store: Arc<mdview_core::notify_store::NotifyStore>,
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
             authentication at all — anyone who can reach this port can read \
             every indexed file, each project's filesystem path, and drive the \
             agent terminal. Bind 127.0.0.1 unless you intend LAN exposure.",
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
        // toa-1 (D5/D11): the method-mismatch-oracle this family used to
        // close with an extra method-checking extractor existed only to
        // keep an unauthenticated `GET` from confirming the route existed
        // via a `405`. With no authentication left to protect (D1), that
        // disguise is gone — every route below is mounted with its one
        // true method, same as any other route in this router. toa-3: the
        // login and rotation routes this family used to carry are gone
        // entirely (D1) — nothing was ever gated on them but themselves.
        .route("/api/terminal-config", post(update_terminal_config))
        .route("/api/projects/:id/unregister", post(unregister_project))
        // D7/D8: the add-project form's target. Mounted with its one true
        // method (toa-1), so an unauthenticated GET here answers axum's
        // ordinary 405 rather than reaching the handler.
        .route("/api/projects/register", post(register_project))
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
        // Gated (D4/D7/D12): `terminal_family_enabled` is the only check
        // left in front of this route.
        .route("/p/:id/_terminal", get(terminal_page))
        // terminal-pane-scope D4: one pane's own page, chosen from the pane
        // strip `terminal_page` renders. The literal `pane/` segment is
        // deliberate — without it, a pane id equal to the literal string
        // `create` would shadow `/p/:id/_terminal/create/pane` and
        // `/create/agent` below.
        .route("/p/:id/_terminal/pane/:pane_id", get(terminal_page_for_pane))
        // agent-terminal-16 (D9): the Transcript tab — a second tab beside
        // Terminal, not a toggle inside its frame.
        .route("/p/:id/_transcript", get(transcript_page))
        // terminal-pane-scope D4: the Transcript tab's own per-pane page,
        // mirroring `terminal_page_for_pane` above.
        .route("/p/:id/_transcript/pane/:pane_id", get(transcript_page_for_pane))
        // agent-terminal-6: one pane's polled screen.
        .route("/p/:id/_terminal/:pane_id/screen", get(terminal_screen))
        // agent-terminal-16 (D9): the gap-free activity channel beside the
        // screen above — same D2 containment boundary, applied via
        // `project_pane_cwd_in_boundary` rather than
        // `project_and_verify_pane_in_boundary` since this route needs the
        // pane's own cwd value, not just a membership check.
        .route("/p/:id/_terminal/:pane_id/transcript", get(terminal_transcript))
        // agent-terminal-9 (D3): the write side — free text and named keys
        // into a pane.
        .route("/p/:id/_terminal/:pane_id/input", post(terminal_input))
        .route("/p/:id/_terminal/:pane_id/keys", post(terminal_keys))
        // agent-terminal-13 (D8/P4): start a new pane or agent in this
        // project — the same D2 containment boundary the routes above use,
        // applied to the destination workspace's own anchor rather than an
        // already-listed pane id, so a request can never start a process in
        // a project it is not looking at.
        .route("/p/:id/_terminal/create/pane", post(terminal_create_pane))
        .route("/p/:id/_terminal/create/agent", post(terminal_create_agent))
        // agent-terminal-10 (D5): the Unassigned group — panes under no
        // registered project's root. Deliberately mounted outside `/p/:id/`
        // (never `/p/unassigned/...`): a registered project's own slug can
        // legitimately be the literal string "unassigned" (`slug_from_root`
        // has no reserved-word exclusion), so nesting this under the
        // project path shape would make that real project's own terminal
        // route ambiguous with this group's route.
        .route("/_terminal/unassigned", get(unassigned_terminal_page))
        .route(
            "/_terminal/unassigned/:pane_id/screen",
            get(unassigned_terminal_screen),
        )
        .route(
            "/_terminal/unassigned/:pane_id/input",
            post(unassigned_terminal_input),
        )
        .route(
            "/_terminal/unassigned/:pane_id/keys",
            post(unassigned_terminal_keys),
        )
        .route("/p/:id/*path", get(project_path))
        .with_state(state)
}

/// D1/D6: the bound on `index_page`'s one herdr snapshot. `SocketHerdr::call`
/// (`herdr/socket.rs:198-217`) carries no timeout of its own on connect,
/// write, or read, and the router has no `TimeoutLayer` — the moment `/`
/// starts making a herdr call at all, an accepted-but-silent socket would
/// otherwise wedge the home page for every visitor. A couple of seconds is
/// long enough for a live daemon's ordinary reply and short enough that a
/// hung one never reads as the page itself being down.
const INDEX_HERDR_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);

/// D9a's pre-flight budget for a root submitted to `register_project`:
/// refuses before indexing starts, rather than after `ensure_project`'s
/// inline, uncapped walk (`indexer.rs:88-107`) has already read the whole
/// tree into sqlite. `REGISTER_MAX_MARKDOWN_FILES` sits well above this
/// repository's own markdown count (~200 files, `docs/history/projects-home/plan.md`
/// § Discovery) so an ordinary project registers without friction, and low
/// enough that the walk itself stays fast; `REGISTER_SCAN_BUDGET` is the
/// same couple-of-seconds order of magnitude as `INDEX_HERDR_SNAPSHOT_TIMEOUT`
/// above, for the same reason — long enough for an ordinary tree, short
/// enough that a pathological one never reads as the daemon being down.
const REGISTER_MAX_MARKDOWN_FILES: usize = 500;
const REGISTER_SCAN_BUDGET: Duration = Duration::from_secs(2);

struct RegisterFlag {
    /// D10's fixed error code from a refused `register_project` redirect —
    /// never the submitted path (see `views::register_error_message`).
    register_error: Option<String>,
}

// `#[derive(Deserialize)]`'s generated struct visitor refuses a repeated
// query key outright ("duplicate field"), and axum's `Query` extractor turns
// that refusal into a 400 for the whole request — so
// `/?register_error=a&register_error=b` turned `/` itself into a 400 rather
// than rendering the Projects page. Nothing about this query string is
// trusted input to begin with (D10: only a fixed code, never the submitted
// path, ever reaches the page), so there is no reason a merely-repeated key
// should fail closed at the extractor. A hand-written `Visitor` walks every
// `(key, value)` pair itself and keeps the last `register_error` seen
// instead of erroring on the second one.
impl<'de> serde::Deserialize<'de> for RegisterFlag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RegisterFlagVisitor;

        impl<'de> serde::de::Visitor<'de> for RegisterFlagVisitor {
            type Value = RegisterFlag;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "a query string carrying at most one register_error value")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut register_error = None;
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if key == "register_error" {
                        register_error = Some(value);
                    }
                }
                Ok(RegisterFlag { register_error })
            }
        }

        deserializer.deserialize_map(RegisterFlagVisitor)
    }
}

async fn index_page(State(st): State<AppState>, Query(flag): Query<RegisterFlag>) -> Response {
    match st.engine.list_projects() {
        Ok(projects) => {
            // D1/D1a/D2/D5/D6: badges are gated on the same single switch as
            // every other terminal route (`terminal_family_enabled`) — off,
            // this makes no herdr call at all, so a switched-off page reads
            // exactly as it did before this feature. On, one snapshot is
            // taken for the whole page and matched against every project in
            // one pass, rather than one herdr round trip per row.
            let badges_enabled = terminal_family_enabled(&st);
            let snapshot = if badges_enabled {
                match tokio::time::timeout(INDEX_HERDR_SNAPSHOT_TIMEOUT, st.herdr.snapshot()).await
                {
                    Ok(Ok(snapshot)) => Some(snapshot),
                    // D6: an errored or a timed-out snapshot both answer with
                    // plain rows — never a raw error, and never a hang.
                    Ok(Err(_)) | Err(_) => None,
                }
            } else {
                None
            };
            // project-suggestions S1/S3: gated on `terminal_family_enabled`
            // alone (the plan's locked narrowing of toa-4/D9's scope for
            // this one surface) — deliberately not also on
            // `unassigned_group_enabled`, which every other reader of
            // `unassigned_panes`'s complement checks. With the switch off,
            // `snapshot` is `None` and this is `Vec::new()` with no herdr
            // call and no filesystem path in reach of the page at all.
            let suggestions: Vec<views::ProjectSuggestion> = snapshot
                .as_ref()
                .map(|snap| suggested_projects(snap, &projects))
                .unwrap_or_default();
            let with_counts: Vec<_> = projects
                .into_iter()
                .map(|p| {
                    let c = st.engine.file_count(&p.id).unwrap_or(0);
                    // D1/D2/D5: the same per-project idiom
                    // `terminal_page_inner` uses (server.rs:747-749) — a
                    // boundary that fails to construct (e.g. a root sitting
                    // on the hard-deny list) empties only *this* row's own
                    // badges, never every row's. That's deliberately not
                    // `unassigned_panes`'s shape, which fails the whole
                    // group closed on any single project's unconstructable
                    // boundary — the wrong semantics for a per-row badge.
                    let panes = snapshot
                        .as_ref()
                        .map(|snap| {
                            mdview_core::paths_boundary::Boundary::new(vec![p.root_path.clone()])
                                .map(|boundary| project_panes(snap, &boundary))
                                .unwrap_or_default()
                        })
                        .unwrap_or_default();
                    (p, c, panes)
                })
                .collect();
            // D5/D4: presence only, never contents — this unauthenticated
            // route reads only the D7 and, per toa-4/D9, the group's own
            // switch (no herdr call, no session), so it can never learn
            // whether any pane is actually unassigned. toa-4: once the
            // group is off by policy — its own switch, not merely an empty
            // pane list — this marker itself becomes a disclosure ("this
            // machine has a host-wide pane group configured"), so it must
            // track both switches, not the family switch alone.
            let unassigned_visible = badges_enabled && unassigned_group_enabled(&st);
            Html(views::project_list_page(
                &with_counts,
                unassigned_visible,
                &suggestions,
                flag.register_error.as_deref(),
            ))
            .into_response()
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
    let notify_credential_view = current_notify_credential_view(&st);
    Html(views::settings_page(
        &cfg,
        flag.saved.is_some(),
        flag.notify_error.is_some(),
        notify_credential_view,
    ))
    .into_response()
}

/// The Telegram credential state `settings_page` renders — masked to the
/// last four characters, or "never saved". This is the only view of the
/// credential any response ever carries, including the one immediately
/// after `update_terminal_config` saves it — there is no reveal-once
/// counterpart anywhere in this file (that mechanism belonged to the
/// terminal token, removed with `terminal_auth`, toa-3, D1).
fn current_notify_credential_view(st: &AppState) -> views::NotifyCredentialView {
    let cred_path =
        mdview_core::config::notify_credential_path_override(st.config_data_dir.as_deref());
    match mdview_core::config::masked_notify_credential(&cred_path) {
        Some(masked) => views::NotifyCredentialView::Masked(masked),
        None => views::NotifyCredentialView::NotConfigured,
    }
}

#[derive(serde::Deserialize, Default)]
struct TerminalConfigJson {
    /// D10: JSON booleans, not a checkbox's presence/absence — the client
    /// (`assets/app.js`) always sends all three, `#[serde(default)]` only
    /// guards a hand-built or partial body against a hard parse error.
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    supervisor_enabled: bool,
    #[serde(default)]
    notify_enabled: bool,
    /// toa-4 (D9): the Unassigned group's own switch, ANDed with `enabled`
    /// above at every unassigned route (`unassigned_group_enabled`) —
    /// turning this on alone opens nothing while `enabled` stays off.
    /// `#[serde(default)]` matches the other switches on this struct: a
    /// hand-built or partial JSON body that omits this key is refused
    /// safely (reads as off) rather than with a hard parse error.
    #[serde(default)]
    unassigned_enabled: bool,
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
/// route rather than a field on `SettingsForm`/`update_config`, so that a
/// `POST /api/config` submission (whose form has no such fields at all —
/// see `SettingsForm`) can never touch them. toa-1 (D1): this route no
/// longer requires a live terminal session to reach — it never gated
/// `terminal_family_enabled` either, since it must stay reachable to turn
/// the switch back on (see `terminal_family_enabled`'s doc).
///
/// D10: the body must be JSON. `Json<TerminalConfigJson>` rejects anything
/// else — including an `application/x-www-form-urlencoded` submission —
/// before this function body ever runs, so a form-encoded POST changes no
/// switch. That matters because a form-encoded POST is a CORS *simple*
/// request: no preflight, and this server has no CORS layer, so with D1's
/// session gone, any page the owner happens to have open could otherwise
/// submit one cross-site carrying the owner's own Cloudflare Access cookie.
/// A JSON body is not a simple request — it forces a preflight this server
/// never answers — so the browser refuses to send it cross-origin at all.
/// `assets/app.js` submits the settings page's terminal form as JSON via
/// `fetch` for exactly this reason.
///
/// toa-4 (D9): `form.unassigned_enabled` is saved the same unconditional
/// way as the other three switches — a body that omits the key is not
/// "leave it alone", it is "off" (`TerminalConfigJson`'s `#[serde(default)]`
/// doc). That fail-closed reading is deliberate for this switch: it is the
/// gate on every herdr pane on the host that lives outside a registered
/// project, so an ambiguous or partial write must never be read as "on".
async fn update_terminal_config(State(st): State<AppState>, Json(form): Json<TerminalConfigJson>) -> Response {
    let config_path = mdview_core::config::config_path_override(st.config_data_dir.as_deref());
    let mut cfg = mdview_core::Config::load_from(&config_path);
    cfg.terminal.enabled = form.enabled;
    cfg.terminal.supervisor_enabled = form.supervisor_enabled;
    cfg.terminal.notify_enabled = form.notify_enabled;
    cfg.terminal.unassigned_enabled = form.unassigned_enabled;
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

#[derive(serde::Deserialize)]
struct RegisterProjectForm {
    path: String,
}

/// D7/D8/D9a/D10: register a project from the Projects page's add-project
/// form. `Engine::register` (`ensure_project`) validates nothing of its
/// own — it canonicalizes with a raw-path fallback (`engine.rs:44-46`),
/// swallows every metadata error into a silently skipped file
/// (`indexer.rs:43-52`), and returns the existing project on a root match
/// rather than failing — so every one of D10's refusals, and D9a's
/// deny-list/cap guard, is this handler's own work, run in
/// `validate_register_path`'s fixed order, each with its own fixed error
/// code. Like every route here it is unauthenticated.
async fn register_project(
    State(st): State<AppState>,
    Form(form): Form<RegisterProjectForm>,
) -> Response {
    // D9a/P1: `validate_register_path` itself canonicalizes, stats, resolves
    // the deny list, and runs the sqlite duplicate lookup, and on success
    // walks the tree up to `REGISTER_SCAN_BUDGET` (2s) — every bit of that is
    // filesystem or sqlite work, and this route is unauthenticated. Running
    // it (and the register call it gates) inline on the async request thread
    // would let concurrent POSTs each pin a tokio worker for up to two
    // seconds and stall every other route including `/`; the whole
    // validate-then-register sequence runs in one `spawn_blocking` so none of
    // it ever touches the async thread.
    let engine = st.engine.clone();
    let code: Result<(), &'static str> = tokio::task::spawn_blocking(move || {
        let canonical = validate_register_path(&engine, &form.path)?;
        // ensure_project indexes inline (engine.rs:72-77), which is exactly
        // the walk D9a exists to keep off the request thread — the
        // pre-flight above only decided whether to index at all.
        //
        // Every named D10 refusal already returned above; a failure here is
        // the engine/store call itself failing after every guard passed, so
        // it gets its own generic code rather than being folded into one of
        // the named ones.
        engine
            .register(&canonical, None)
            .map(|_| ())
            .map_err(|_| "failed")
    })
    .await
    .unwrap_or(Err("failed"));

    match code {
        Ok(()) => Redirect::to("/").into_response(),
        Err(code) => Redirect::to(&format!("/?register_error={code}")).into_response(),
    }
}

/// The ordered D9a/D9b/D10 gate a submitted path must pass before
/// `Engine::register` ever runs. Fail-closed throughout: any ambiguity is a
/// refusal, never a best-effort accept. Returns the canonical path on
/// success, so `register_project` never re-resolves it, or one of D10's
/// fixed error codes on refusal — never the raw submitted string, which
/// `views::register_error_message` never receives either. Filesystem and
/// sqlite calls throughout (canonicalize, the deny-list check, the
/// duplicate lookup, the bounded scan) make this a blocking function —
/// callers run it inside `spawn_blocking`, never directly on an async task.
fn validate_register_path(engine: &Engine, raw: &str) -> Result<PathBuf, &'static str> {
    let raw_path = std::path::Path::new(raw);
    // Empty, relative, and any raw `.`/`..` component are refused before
    // ever touching the filesystem. This mirrors
    // `paths_boundary::reject_traversal`'s own check (not exposed publicly,
    // and this cell's prohibitions rule out duplicating `hard_deny_list` —
    // this is the separate, unrelated traversal-component gate, not the
    // deny list) — canonicalizing first would silently resolve a `..` away
    // rather than refuse the submission that carried one.
    let has_traversal = raw_path.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    });
    if raw.trim().is_empty() || !raw_path.is_absolute() || has_traversal {
        return Err("invalid_path");
    }
    let canonical = std::fs::canonicalize(raw_path).map_err(|_| "not_found")?;
    if !canonical.is_dir() {
        return Err("not_directory");
    }
    // D9a/D9b: the deny-list, absolute-path, and traversal gate all in one
    // call — refuses a root on `paths_boundary::hard_deny_list` (e.g.
    // `$HOME/.ssh`) without this handler ever seeing, let alone
    // duplicating, that list (`paths_boundary.rs:101-125`).
    // `Boundary::new` alone only answers "is this root inside a denied
    // root" (its `check_denied`/`is_within_allowed` machinery is built for
    // an already-constructed boundary's *allowed* side), so a root that
    // *contains* a denied directory — `$HOME` containing `$HOME/.ssh`, or
    // `/` and `/home` containing it transitively — passed that gate alone
    // and went on to index (and later serve, unauthenticated) markdown
    // under `~/.ssh`, `~/.aws` and `~/.gnupg`. `is_denied_root` closes the
    // other direction; both run so neither guard's own coverage regresses
    // silently if the other is ever removed.
    if mdview_core::paths_boundary::Boundary::new(vec![canonical.clone()]).is_err()
        || mdview_core::paths_boundary::is_denied_root(&canonical)
    {
        return Err("denied");
    }
    // D10: look up the CANONICAL path, never the raw submitted string — a
    // symlink to, or a trailing-slash form of, an already-registered root
    // canonicalizes to the same value and must be caught here too, rather
    // than falling through to `ensure_project` as a silent success. A store
    // failure here is not a path decision at all — it gets the same generic
    // `"failed"` code `register_project` uses for `Engine::register`'s own
    // failure, never folded into `"denied"` (see this function's own doc).
    match engine.store.find_project_by_root(&canonical) {
        Ok(Some(_)) => return Err("duplicate"),
        Ok(None) => {}
        Err(_) => return Err("failed"),
    }
    // D9a: bound the work before doing it — the same `WalkBuilder` settings
    // `scan_markdown_files` uses, aborted at a file cap or a wall-clock
    // budget rather than walked to completion. The two bounds get distinct
    // codes: folding them into one `"too_large"` would let a route-level
    // test meant to prove the *file* cap pass instead via a slow walk
    // crossing the *time* budget, without ever proving the cap it claims to.
    match mdview_core::indexer::bounded_scan_markdown_files(
        &canonical,
        &engine.config.indexing.exclude_patterns,
        REGISTER_MAX_MARKDOWN_FILES,
        REGISTER_SCAN_BUDGET,
    ) {
        mdview_core::indexer::ScanBudget::Ok(_) => Ok(canonical),
        mdview_core::indexer::ScanBudget::TooManyFiles => Err("too_large"),
        mdview_core::indexer::ScanBudget::TooSlow => Err("too_slow"),
    }
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

/// terminal-pane-scope D4: which pane a bare `/_terminal` or `/_transcript`
/// request opens, since the page now renders exactly one. The snapshot's
/// global `focused_pane_id` when that pane is one of this project's own
/// (`panes`); otherwise the first pane in this project's own list order —
/// never a different project's focus, and never a pane this project's own
/// D2 boundary did not already accept. `None` only when `panes` is empty,
/// which is the honest empty state, not a redirect to a pane that does not
/// exist.
fn default_pane_id(panes: &[views::TerminalPaneView], focused_pane_id: Option<&str>) -> Option<String> {
    if let Some(focused) = focused_pane_id {
        if panes.iter().any(|p| p.pane_id == focused) {
            return Some(focused.to_string());
        }
    }
    panes.first().map(|p| p.pane_id.clone())
}

/// `GET /p/:id/_terminal` and `GET /p/:id/_terminal/pane/:pane_id` (D2/D4/D6/
/// D12) — one pane's own page, open to anyone who reaches the daemon (D1).
/// `terminal_family_enabled` is the only gate left: off, this answers with
/// the ordinary not-found page, same as an unregistered project id below —
/// that truth is about the *route* existing, not about any particular
/// project id being valid.
///
/// A silent or errored herdr socket renders the D6 remedy state — never a
/// raw error, and never an empty pane list that would look identical to a
/// project that genuinely has zero agents running.
///
/// `requested_pane_id` is `None` for the bare route (`terminal_page`, which
/// falls back through [`default_pane_id`]) and `Some` for the pane-scoped
/// route (`terminal_page_for_pane`) — a pane id absent from this project's
/// own D2 boundary-filtered list answers the ordinary not-found page, the
/// same refusal `terminal_screen` makes, and never names the pane it
/// refused.
async fn terminal_page_inner(
    st: AppState,
    id: String,
    requested_pane_id: Option<String>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_page();
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
            let selected = match requested_pane_id {
                Some(pane_id) => {
                    if !panes.iter().any(|p| p.pane_id == pane_id) {
                        return not_found("pane not found");
                    }
                    Some(pane_id)
                }
                None => default_pane_id(&panes, snapshot.focused_pane_id.as_deref()),
            };
            Html(views::terminal_page(&project, &panes, selected.as_deref(), &presets)).into_response()
        }
        Err(_) => Html(views::terminal_down_page(&project)).into_response(),
    }
}

async fn terminal_page(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    terminal_page_inner(st, id, None).await
}

async fn terminal_page_for_pane(
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
) -> Response {
    terminal_page_inner(st, id, Some(pane_id)).await
}

/// `GET /p/:id/_transcript` and `GET /p/:id/_transcript/pane/:pane_id`
/// (D2/D4/D6/D9/D12) — the Transcript tab: the same per-pane page shape
/// `terminal_page_inner` renders, with a transcript viewport per selected
/// pane instead of a screen. `assets/app.js`'s transcript poller fills it in
/// from `terminal_transcript` below. Guarded and constructed identically to
/// `terminal_page_inner` — same D7 switch, same herdr snapshot + D2
/// boundary, same D6 herdr-down page, same pane selection and the same
/// not-found refusal for a pane outside this project — because listing
/// *which* panes belong to this project still requires reaching herdr, even
/// though the transcript content itself never does (D9: the transcript is
/// the agent's own on-disk log, read directly, not through herdr). No
/// creation controls here (D8 stays on the Terminal tab only); this tab is
/// read-only.
async fn transcript_page_inner(
    st: AppState,
    id: String,
    requested_pane_id: Option<String>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_page();
    }
    let Ok(Some(project)) = st.engine.get_project(&id) else {
        return not_found("project not found");
    };
    match st.herdr.snapshot().await {
        Ok(snapshot) => {
            let panes = mdview_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
                .map(|boundary| project_panes(&snapshot, &boundary))
                .unwrap_or_default();
            let selected = match requested_pane_id {
                Some(pane_id) => {
                    if !panes.iter().any(|p| p.pane_id == pane_id) {
                        return not_found("pane not found");
                    }
                    Some(pane_id)
                }
                None => default_pane_id(&panes, snapshot.focused_pane_id.as_deref()),
            };
            Html(views::transcript_page(&project, &panes, selected.as_deref())).into_response()
        }
        Err(_) => Html(views::terminal_down_page(&project)).into_response(),
    }
}

async fn transcript_page(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    transcript_page_inner(st, id, None).await
}

async fn transcript_page_for_pane(
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
) -> Response {
    transcript_page_inner(st, id, Some(pane_id)).await
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
/// `POST /api/terminal-config`, which must stay reachable so the switch can
/// be turned back on. toa-1/toa-3: this is now the *only* gate in front of
/// the terminal family (D2) — the auth and method extractors that used to
/// run ahead of it, and the login/rotation routes that minted and checked
/// them, are gone (D1/D5).
/// The operator's configured display hostname, if any — the same
/// `server.hostname` the settings page writes. `None` leaves a doc link
/// same-origin, which is what the terminal page itself is served from.
fn configured_hostname(st: &AppState) -> Option<String> {
    mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ))
    .server
    .hostname
}

fn terminal_family_enabled(st: &AppState) -> bool {
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    cfg.terminal.enabled
}

/// toa-4 (D9): the Unassigned group's own gate, checked in addition to
/// `terminal_family_enabled` above, never instead of it. This group reaches
/// every herdr pane on the host that is not inside a registered project's
/// root — unrelated repositories, root shells, other people's agents — and
/// it has no containment check of its own (`unassigned_panes`'s doc
/// comment): before D1 removed the terminal's session, that session was
/// what authorized it. With no session left, turning this switch on is the
/// deliberate act D9 requires, and turning off the family switch alone
/// (`enabled`) still closes this group even if this switch stays on — the
/// two are ANDed, not substitutes for one another. `cfg.terminal
/// .unassigned_enabled` defaults to `false` (`TerminalConfig`'s
/// `#[derive(Default)]`), so a config that has never mentioned this key
/// reads as off, not merely the shipped default file.
fn unassigned_group_enabled(st: &AppState) -> bool {
    let cfg = mdview_core::Config::load_from(&mdview_core::config::config_path_override(
        st.config_data_dir.as_deref(),
    ));
    cfg.terminal.unassigned_enabled
}

/// D12's disabled answer for a terminal **page** route (`terminal_page`,
/// `transcript_page`, `unassigned_terminal_page`): mdview's ordinary
/// not-found page — the same `not_found` helper every other page route in
/// this file already answers with — never the typeless empty `404` the old,
/// now-removed `terminal_auth` module's opaque 404 gave, which was
/// indistinguishable from an unrouted path and made a browser download a
/// file instead of showing a page (D2's struck "byte-identical to unrouted"
/// rule).
fn terminal_disabled_page() -> Response {
    not_found("the agent terminal is disabled")
}

/// D12's disabled answer for a terminal **data** route the client polls
/// (`terminal_screen`, `terminal_transcript`, `terminal_input`,
/// `terminal_keys`, `terminal_create_pane`, `terminal_create_agent`, and
/// their Unassigned-group siblings): a `404` carrying a JSON body naming the
/// reason, so a poller reading `response.json()` gets a reason rather than
/// an HTML page or a body it cannot parse.
fn terminal_disabled_json_404() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "the agent terminal is disabled" })),
    )
        .into_response()
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

/// Shared by `terminal_screen` and `unassigned_terminal_screen` — mirrors
/// herdr-go's own `ScreenQuery` (`herdr-go/src/web/screen.rs`). Presence
/// requests older pane content via `PaneScroller::read_history`
/// (`herdr/pane_scroller.rs`) instead of the routes' existing default
/// live-view read; absent leaves today's behavior byte-for-byte unchanged.
/// The value doubles as the cumulative depth (PageUp-hops back from the live
/// bottom) this one call goes before always restoring to live — the gateway
/// keeps no scroll depth of its own between requests, so the client sends
/// one more than its last request to go further back than last time
/// (`assets/app.js`'s own running per-pane counter). A present-but-non-
/// numeric value falls back to one hop rather than erroring, matching
/// herdr-go's own fallback for callers that only ever checked presence.
#[derive(serde::Deserialize, Default)]
struct ScreenQuery {
    #[serde(default)]
    history: Option<String>,
}

/// Parses `ScreenQuery::history` into a PageUp-hop count for
/// `PaneScroller::read_history` — non-numeric falls back to 1, and the
/// result is always clamped to at least 1 (mirrors herdr-go's own
/// `.parse::<usize>().unwrap_or(1).max(1)`).
fn history_pages(history: &str) -> usize {
    history.parse::<usize>().unwrap_or(1).max(1)
}

/// `GET /p/:id/_terminal/:pane_id/screen` (D2/D3/D4/D6) — one pane's current
/// screen, polled by the client in `assets/app.js`. Modeled on herdr-go's
/// `ScreenBody { text, revision }` (`herdr-go/src/web/screen.rs`), but the
/// `text` field now carries safe, escaped HTML rather than raw text
/// (agent-terminal-12): `mdview_core::ansi::to_html` translates herdr's raw
/// ANSI screen into `<span class="ansi-…">` markup server-side — text is
/// HTML-escaped before any markup wraps it, and any escape sequence the
/// translator does not model (cursor movement, OSC titles, …) is dropped
/// rather than ever reaching the page. `revision` is no longer herdr's own
/// field (screen-revision fix): that value only bumps when the operator's
/// own input is echoed back, not when the agent under it produces new
/// output on its own, which froze the client's poller on any pane whose
/// agent was still actively writing. `revision` is now
/// `mdview_core::ansi::revision_of(&read.text)`, a stateless hash of the
/// raw screen text, so it changes exactly when the text does; the client
/// still compares it to skip a redundant repaint.
///
/// Guarded by the D7 enabled switch (D12: a reasoned JSON 404 when off),
/// then the same D2 containment boundary `terminal_page` uses — a pane id is
/// only ever read if it is already present in this project's own
/// boundary-filtered pane list, never trusted from the URL alone. A pane
/// that existed when the page listed it but is gone by the time this fires
/// (or was never in this project) gets the ordinary not-found page, distinct
/// from herdr itself being unreachable.
async fn terminal_screen(
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
    Query(query): Query<ScreenQuery>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_json_404();
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
    let read = if let Some(history) = &query.history {
        // TEMPORARY (remove once the button is confirmed working end to end):
        // proves whether the browser's press reaches this route at all.
        // Carries only the pane id and the hop count — never any typed text
        // or key press.
        tracing::info!("screen history read pane={pane_id} pages={}", history_pages(history));
        let scroller = herdr::pane_scroller::PaneScroller::new(st.herdr.as_ref());
        scroller.read_history(&pane_id, history_pages(history)).await
    } else {
        // Unchanged default behavior: today's existing read, named
        // explicitly.
        st.herdr
            .read_pane(&pane_id, herdr::ReadSource::Recent, SCREEN_READ_LINES)
            .await
    };
    match read {
        Ok(read) => {
            let revision = mdview_core::ansi::revision_of(&read.text);
            // An agent names its own documents constantly; every one of them
            // is a page this same server renders, so the names become links
            // to it. Applied over the translated HTML, never the raw screen —
            // the translation is what made the text safe to embed.
            let html = mdview_core::doc_links::linkify_docs(
                &mdview_core::ansi::to_html(&read.text),
                &mdview_core::doc_links::link_base(&id, configured_hostname(&st).as_deref()),
            );
            Json(json!({ "text": html, "revision": revision })).into_response()
        }
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        // Any other herdr failure (socket gone, protocol mismatch, a
        // request-level error) collapses to the same D6 remedy `terminal_page`
        // renders for a silent socket — the client shows it verbatim rather
        // than a blank screen, and never a raw error type.
        Err(_) => herdr_down_response(),
    }
}

/// How many lines of a pane the screen routes ask herdr for. `Recent` is
/// herdr's own scrollback buffer, so a shell pane answers with its history
/// (measured live: a shell with 423 lines of scrollback returns 200 here
/// against 43 for `Visible`); an alt-screen agent keeps no scrollback of its
/// own — `max_offset_from_bottom` is 0 — so it answers with exactly the same
/// frame `Visible` gave, never less. 200 sits well under herdr's own 1000-line
/// server-side cap (`SocketHerdr::read_pane`) and is what one screen box can
/// hold without the poll growing unbounded.
const SCREEN_READ_LINES: usize = 200;

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

/// `GET /p/:id/_terminal/:pane_id/transcript?cursor=...` (D2/D6/D9/D12) — the
/// gap-free activity channel beside `terminal_screen`'s polled screen.
/// Guarded identically: the D7 enabled switch (D12: a reasoned JSON 404 when
/// off), then the same D2 containment boundary as every other pane-scoped
/// route — via `project_pane_cwd_in_boundary` rather than
/// `project_and_verify_pane_in_boundary`, since this route needs the pane's
/// resolved cwd itself (a request looking at project A must never read an
/// agent's transcript in project B).
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
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
    Query(q): Query<TranscriptQuery>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_json_404();
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
/// Guarded exactly like `terminal_screen`: the D7 enabled switch (D12: a
/// reasoned JSON 404 when off), then the same D2 containment boundary via
/// `project_and_verify_pane_in_boundary`.
async fn terminal_input(
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
    Json(body): Json<ReplyBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_json_404();
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
/// terminal family's disabled-state `404` (this is a validation failure on
/// an already-enabled route, not a disabled-switch refusal).
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
    State(st): State<AppState>,
    Path((id, pane_id)): Path<(String, String)>,
    Json(body): Json<KeysBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_json_404();
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
/// actually called is `409` — never `404`, which stays reserved for a route
/// that genuinely does not exist or a disabled terminal (`terminal_disabled_json_404`)
/// — and everything else collapses to `502` carrying the message, the same
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
/// boundary, so a request can never aim a creation at a workspace outside
/// the project it is looking at. The body is deliberately empty: a shell
/// takes no command, and no `cwd`/`argv`/`env` field is declared to receive
/// anything a client might try to send.
///
/// Guarded exactly like every other terminal route: the D7 enabled switch
/// (D12: a reasoned JSON 404 when off), then the project lookup.
async fn terminal_create_pane(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(_body): Json<CreatePaneBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_json_404();
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
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CreateAgentBody>,
) -> Response {
    if !terminal_family_enabled(&st) {
        return terminal_disabled_json_404();
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

/// terminal-pane-scope's D1/D2: membership is decided over `snapshot.panes`
/// — the set this function is actually about — not `snapshot.agents`, since
/// an agent is only a subset of the panes that legitimately belong to a
/// project (agent-terminal's own D2 names it: "the herdr **panes** whose
/// working directory sits under that project's `root_path`" — pane
/// iteration was the decision's own wording all along). A pane qualifies
/// when the D2 containment boundary accepts its `cwd`; when `cwd` is absent
/// or the boundary refuses it, the boundary is tried again against
/// `foreground_cwd`. `cwd` wins whenever both would validate: the path this
/// function returns is not display-only — `project_pane_cwd_in_boundary`
/// hands it to `read_activity`, and `mdview_core::transcript` uses it as the
/// transcript directory selector, so preferring the live-but-volatile
/// `foreground_cwd` would silently re-key an existing pane's transcript away
/// from the directory it actually launched in. `foreground_cwd` is
/// unix-only and always `None` elsewhere (`herdr::wire::Pane`'s doc), so
/// this is a no-op on every other platform. The agent, if any, is then
/// joined by `pane_id` — present, it carries today's
/// `kind`/`name`/`status`/`title`; absent, the pane is a real shell row (D2)
/// rather than an absence, with a `kind` that says so instead of borrowing
/// an agent's vocabulary. The boundary itself does the actual decision
/// (symlink resolution, component-wise containment, fail-closed on any
/// ambiguity) for both directories alike.
fn project_panes(
    snapshot: &herdr::Snapshot,
    boundary: &mdview_core::paths_boundary::Boundary,
) -> Vec<views::TerminalPaneView> {
    snapshot
        .panes
        .iter()
        .filter_map(|pane| {
            let resolved = pane
                .cwd
                .as_deref()
                .and_then(|raw| boundary.validate_existing(std::path::Path::new(raw)).ok())
                .or_else(|| {
                    pane.foreground_cwd.as_deref().and_then(|raw| {
                        boundary.validate_existing(std::path::Path::new(raw)).ok()
                    })
                })?;
            let agent = snapshot.agents.iter().find(|a| a.pane_id == pane.pane_id);
            Some(views::TerminalPaneView {
                pane_id: pane.pane_id.clone(),
                kind: agent
                    .map(|a| a.kind.clone())
                    .unwrap_or_else(|| "shell".to_string()),
                name: agent.map(|a| a.name.clone()).unwrap_or_default(),
                // A shell row (no agent) admits its own status rather than
                // borrowing an `AgentStatus` it does not have (D2/D3) — the
                // card's status pill names it "shell" instead of reading
                // blank, the same way `kind` already does.
                status: agent
                    .map(|a| a.status.as_str().to_string())
                    .unwrap_or_else(|| "shell".to_string()),
                title: agent.map(|a| a.title.clone()).unwrap_or_default(),
                cwd: resolved.to_string_lossy().into_owned(),
                workspace: snapshot.workspace_label_for_id(&pane.workspace_id),
                tab: snapshot.tab_label_for_id(&pane.tab_id),
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
/// the exact widening this group's own gate is the last line of defense
/// against (per P6, there is no second containment check for this group;
/// toa-4/D9's `unassigned_enabled` switch, checked by every caller of this
/// function, is what authorizes it now that D1 removed the session that
/// used to). There is no way to tell, without a working boundary, which of
/// the project's real panes those were — so the whole group fails closed to
/// empty rather than guess, the same "fail closed to zero, not a crash and
/// not a laxer check" rule `terminal_page` already applies to a single
/// unconstructible project.
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
                workspace: snapshot.workspace_label_for(agent),
                tab: snapshot.tab_label_for(agent),
            }
        })
        .collect()
}

/// project-suggestions S1: every folder where a herdr **pane** (not just an
/// agent — `unassigned_panes` iterates `snapshot.agents`, which is blind to
/// a plain shell holding no agent at all) is running that sits under no
/// registered project's own D2 containment boundary. Reads the same
/// complement `unassigned_panes` computes, over `snapshot.panes` instead, so
/// a folder that only ever launched a shell is still surfaced as something
/// worth registering.
///
/// Mirrors `unassigned_panes`'s fail-closed rule exactly, for the identical
/// reason given there: without a working `Boundary` there is no way to tell
/// a registered project's own panes apart from a genuinely unregistered one,
/// so one unconstructable root empties the whole suggestion list rather than
/// guess.
///
/// S2: a suggestion's path is the pane's own `cwd`, exactly as herdr reports
/// it — never resolved through a `Boundary`, never walked up to a repository
/// root. A pane with no cwd at all is dropped rather than surfaced as a
/// blank path, and a directory that is exactly a registered project's own
/// root is dropped too (a pane can sit in that project's *parent*, which is
/// a genuinely different, unregistered folder, and must still be
/// suggested). Suggestions are deduplicated by directory and carry the
/// number of sessions found there, in first-seen order.
fn suggested_projects(
    snapshot: &herdr::Snapshot,
    projects: &[mdview_core::domain::Project],
) -> Vec<views::ProjectSuggestion> {
    let mut assigned: std::collections::HashSet<String> = std::collections::HashSet::new();
    let registered_roots: std::collections::HashSet<String> = projects
        .iter()
        .map(|p| p.root_path.to_string_lossy().into_owned())
        .collect();
    for p in projects {
        match mdview_core::paths_boundary::Boundary::new(vec![p.root_path.clone()]) {
            Ok(boundary) => {
                assigned.extend(project_panes(snapshot, &boundary).into_iter().map(|pane| pane.pane_id));
            }
            Err(_) => {
                // Fail closed, the same reason `unassigned_panes` gives:
                // without a working boundary this project's own panes
                // cannot be told apart from a genuinely unregistered one,
                // so the whole suggestion list empties rather than risk a
                // wrong complement.
                return Vec::new();
            }
        }
    }

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for pane in &snapshot.panes {
        if assigned.contains(&pane.pane_id) {
            continue;
        }
        let Some(cwd) = pane.cwd.as_deref().filter(|c| !c.is_empty()) else {
            continue;
        };
        if registered_roots.contains(cwd) {
            continue;
        }
        if !counts.contains_key(cwd) {
            order.push(cwd.to_string());
        }
        *counts.entry(cwd.to_string()).or_insert(0) += 1;
    }

    order
        .into_iter()
        .map(|path| {
            let session_count = counts[&path];
            let name = std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            views::ProjectSuggestion {
                path,
                name,
                session_count,
            }
        })
        .collect()
}

/// `GET /_terminal/unassigned` (D5/D6/D9/D12) — the cross-project pane
/// list. Guarded by both the D7 family switch and, per toa-4/D9, the
/// group's own `unassigned_group_enabled` switch (D12: the ordinary
/// not-found page when either is off) — every registered project's own
/// boundary check happens inside `unassigned_panes`, not here. A silent
/// herdr socket renders the same D6 remedy `terminal_page` uses.
///
/// agent-terminal-11: a registry read failure used to fall through
/// `unwrap_or_default()` to an empty project list, which made every pane in
/// the whole snapshot — including ones that plainly belong to a registered
/// project — read as unassigned. Fail closed instead: an unreadable
/// registry renders the group empty, the same as `unassigned_panes` failing
/// closed on an unconstructable project boundary.
async fn unassigned_terminal_page(State(st): State<AppState>) -> Response {
    if !terminal_family_enabled(&st) || !unassigned_group_enabled(&st) {
        return terminal_disabled_page();
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

/// `GET /_terminal/unassigned/:pane_id/screen` (D5/D6/D9/D12) — one
/// unassigned pane's current screen, the same shape `terminal_screen`
/// returns for a project's own pane. Guarded identically: the D7 switch
/// and, per toa-4/D9, the group's own switch (D12: a reasoned JSON 404
/// when either is off), then `verify_pane_is_unassigned`.
async fn unassigned_terminal_screen(
    State(st): State<AppState>,
    Path(pane_id): Path<String>,
    Query(query): Query<ScreenQuery>,
) -> Response {
    if !terminal_family_enabled(&st) || !unassigned_group_enabled(&st) {
        return terminal_disabled_json_404();
    }
    if let Err(refusal) = verify_pane_is_unassigned(&st, &pane_id).await {
        return refusal;
    }
    let read = if let Some(history) = &query.history {
        // TEMPORARY (remove once the button is confirmed working end to end):
        // proves whether the browser's press reaches this route at all.
        // Carries only the pane id and the hop count — never any typed text
        // or key press.
        tracing::info!("screen history read pane={pane_id} pages={}", history_pages(history));
        let scroller = herdr::pane_scroller::PaneScroller::new(st.herdr.as_ref());
        scroller.read_history(&pane_id, history_pages(history)).await
    } else {
        // Unchanged default behavior: today's existing read, named
        // explicitly.
        st.herdr
            .read_pane(&pane_id, herdr::ReadSource::Recent, SCREEN_READ_LINES)
            .await
    };
    match read {
        Ok(read) => {
            let revision = mdview_core::ansi::revision_of(&read.text);
            Json(json!({ "text": mdview_core::ansi::to_html(&read.text), "revision": revision }))
                .into_response()
        }
        Err(herdr::HerdrError::NoSuchPane(_)) => not_found("pane not found"),
        Err(_) => herdr_down_response(),
    }
}

/// `POST /_terminal/unassigned/:pane_id/input` (D3/D5/D9) — the Unassigned
/// group's write path from agent-terminal-9: free-text reply, same
/// `ReplyBody { text, submit }` shape and the same send≠submit semantics as
/// `terminal_input`. Guarded identically, plus toa-4/D9's own group switch,
/// via `verify_pane_is_unassigned`.
async fn unassigned_terminal_input(
    State(st): State<AppState>,
    Path(pane_id): Path<String>,
    Json(body): Json<ReplyBody>,
) -> Response {
    if !terminal_family_enabled(&st) || !unassigned_group_enabled(&st) {
        return terminal_disabled_json_404();
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

/// `POST /_terminal/unassigned/:pane_id/keys` (D3/D5/D9) — the Unassigned
/// group's other write path from agent-terminal-9: named key presses, same
/// `KeysBody { keys }` shape as `terminal_keys`. Guarded identically, plus
/// toa-4/D9's own group switch, via `verify_pane_is_unassigned`.
async fn unassigned_terminal_keys(
    State(st): State<AppState>,
    Path(pane_id): Path<String>,
    Json(body): Json<KeysBody>,
) -> Response {
    if !terminal_family_enabled(&st) || !unassigned_group_enabled(&st) {
        return terminal_disabled_json_404();
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

    // bbp-11: the by-phase board links every entry of `phase_board` to this
    // route, including a feature whose lane record survives with zero live
    // cells (routine in a real store — cells archive after a feature
    // closes; `counter-teeth`/`gate-door-refusal` in beehive's own store are
    // exactly this shape). Before bbp-11 this route only recognized a
    // feature through its live D7 buckets or `shipped`, so every phase card
    // for a lane-only feature 404'd — found by loading the rendered board,
    // not by the suite. `phase_board` is the same "known feature" set the
    // board itself just placed a card for, so a feature this route now
    // shows a card for must resolve here too.
    let known_feature = shipped.is_some()
        || !buckets.doing.is_empty()
        || !buckets.waiting.is_empty()
        || !buckets.stuck.is_empty()
        || !buckets.done.is_empty()
        || snapshot.phase_board.iter().any(|f| f.feature == feature);
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
            // A fresh FakeHerdr per state — no route test ever reaches a
            // real herdr socket. Tests that need specific panes replace this
            // with their own `FakeHerdr` (see the `terminal_route_*` tests
            // below in this module).
            herdr: Arc::new(crate::herdr::fake::FakeHerdr::new()),
            // No route test reads the real `~/.claude/projects` by default —
            // transcript tests set this explicitly (see
            // `transcript_root_dir` below).
            transcript_root: None,
            // A fresh manager per state — no route test shares live
            // background tasks across states.
            terminal_background: Arc::new(crate::TerminalBackground::new()),
            // In-memory outbox — no route test ever touches a real sqlite
            // file for this.
            notify_store: Arc::new(
                mdview_core::notify_store::NotifyStore::open_in_memory().unwrap(),
            ),
        }
    }

    /// `build_state()` plus `config_data_dir` pointed at the scratch `dir`
    /// — every settings/terminal route resolves `config.toml` and the
    /// notify credential file through this override, so a test that leaves
    /// it unset would silently read/write the real `~/.mdview` instead.
    fn build_state_with_dir(dir: &Path) -> AppState {
        let mut st = build_state();
        st.config_data_dir = Some(dir.to_path_buf());
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

    /// (happy, bbp-11) The active feature's phase card states its progress
    /// as a plain `done/total` count over its own live (non-dropped) cells —
    /// replaces `happy_path_returns_200_with_bucket_counts`, which asserted
    /// the four now-retired `data-bucket` attributes.
    #[tokio::test]
    async fn happy_path_returns_200_with_phase_board_progress() {
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
        assert!(body.contains("data-phase-board-count=\"1\""), "{body}");
        assert!(
            body.contains("2/5 cells done"),
            "expected the phase card to state 2 of 5 non-dropped cells done: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge, bbp-11) No lane records and no active feature: the by-phase
    /// board renders its own honest empty line, never a hidden section or a
    /// zeroed board — replaces `empty_cells_dir_yields_four_zero_buckets`.
    #[tokio::test]
    async fn empty_store_renders_honest_empty_phase_board() {
        let root = fresh_root("empty-cells");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let st = build_state();
        let project = register(&st, &root, "empty-cells");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("data-phase-board-count=\"0\""), "{body}");
        assert!(
            body.contains("No features are tracked by phase right now."),
            "expected an honest empty state, not a hidden or zeroed board: {body}"
        );

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

    /// A `.bee/lanes/<feature>.json` fixture. `approved_gates` takes a raw
    /// JSON object body (e.g. `Some(r#""context": true, "shape": true"#)`)
    /// so a caller can express exactly the gates it needs; `None` for either
    /// optional param omits the key entirely, matching a lane record that
    /// never carried it (bbp-10: a lane record carries its own
    /// `approved_gates` and `created_at`, not just `feature`/`phase`/`mode`/
    /// `next_action` — this builder must be able to express both).
    fn lane_json(
        feature: &str,
        phase: &str,
        mode: &str,
        next_action: &str,
        approved_gates: Option<&str>,
        created_at: Option<&str>,
    ) -> String {
        let gates_field = approved_gates
            .map(|g| format!(r#", "approved_gates": {{{g}}}"#))
            .unwrap_or_default();
        let created_at_field =
            created_at.map(|c| format!(r#", "created_at": "{c}""#)).unwrap_or_default();
        format!(
            r#"{{"feature": "{feature}", "phase": "{phase}", "mode": "{mode}", "next_action": "{next_action}"{gates_field}{created_at_field}}}"#
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

    /// bee-cockpit-6 / bbp-11 (happy, split from
    /// `panels_render_backlog_sessions_and_lanes_with_liveness`, which
    /// asserted across backlog, sessions and lanes in one body): the backlog
    /// panel states PBI statuses and finding severity counts.
    #[tokio::test]
    async fn backlog_panel_states_pbi_statuses_and_finding_severities() {
        let root = fresh_root("panels-happy-backlog");
        write(
            &root,
            ".bee/backlog.jsonl",
            "{\"kind\":\"pbi\",\"id\":\"PBI-1\",\"title\":\"Add search\",\"status\":\"in-flight\",\"feature\":\"demo\"}\n\
             {\"kind\":\"pbi\",\"id\":\"PBI-2\",\"title\":\"Add filter\",\"status\":\"done\",\"feature\":\"demo\"}\n\
             {\"ts\":\"2026-08-05T04:00:00Z\",\"type\":\"finding\",\"title\":\"Race in write path\",\"detail\":\"d\",\"severity\":\"P1\",\"layer\":\"server\",\"feature\":\"demo\"}\n\
             {\"ts\":\"2026-08-05T03:00:00Z\",\"type\":\"finding\",\"title\":\"Slow query\",\"detail\":\"d\",\"severity\":\"P2\",\"layer\":\"db\",\"feature\":\"demo\"}\n",
        );

        let st = build_state();
        let project = register(&st, &root, "panels-happy-backlog");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("in-flight: 1"), "{body}");
        assert!(body.contains("done: 1"), "{body}");
        assert!(body.contains("P1: 1"), "{body}");
        assert!(body.contains("P2: 1"), "{body}");
        assert!(body.contains("P3: 0"), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 / bbp-11 (happy, split — see above): the sessions panel
    /// states each session's liveness in plain relative language, never a
    /// raw timestamp.
    #[tokio::test]
    async fn sessions_panel_states_liveness_in_plain_language() {
        let root = fresh_root("panels-happy-sessions");
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

        let st = build_state();
        let project = register(&st, &root, "panels-happy-sessions");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("live"), "{body}");
        assert!(body.contains("stale"), "{body}");
        assert!(body.contains("4 minutes ago"), "{body}");
        assert!(body.contains("2 hours ago"), "{body}");
        assert!(!body.contains("T04:"), "raw ISO timestamp leaked into a heartbeat: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 / bbp-11 (happy, split — see above): a lane record
    /// places its feature on the by-phase board at its own recorded phase,
    /// carrying its next action — this is the assertion that used to read
    /// "lanes panel renders the lane record"; `bee_lanes_panel` is retired
    /// (bbp-11), so the same lane data now surfaces through
    /// `bee_phase_board_section` instead. The fixture's workspace
    /// (`ws-1`/`wt/demo`) is deliberately not asserted here: worktree
    /// workspace rendering is retired with `bee_lanes_panel` and has no
    /// replacement in this cell — the plan (S4) folds it into the Sessions
    /// panel as its own later slice.
    #[tokio::test]
    async fn phase_board_places_lane_feature_at_its_recorded_phase() {
        let root = fresh_root("panels-happy-lanes");
        write(&root, ".bee/lanes/demo.json", &lane_json("demo", "swarming", "standard", "run tests", None, None));

        let st = build_state();
        let project = register(&st, &root, "panels-happy-lanes");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-phase-board-count=\"1\""), "{body}");
        assert!(body.contains("data-phase-col=\"swarming\""), "{body}");
        assert!(body.contains("run tests"), "{body}");
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/feature/demo\"", project.id)),
            "the phase card must link to the feature detail page: {body}"
        );

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

    /// (bbp-14, edge) 25 PBIs exceed the backlog panel's own display cap
    /// (20), so the visible title list must state its true total (25)
    /// alongside the capped subset actually shown — the status count chips
    /// above it are computed over the full, uncapped set and must still
    /// read 25, never 20. Found by rendering `beehive`'s real 123-PBI store
    /// against an early, uncapped draft of this list, which turned the
    /// panel into exactly the unreadable per-item dump its own status chips
    /// exist to avoid.
    #[tokio::test]
    async fn capped_backlog_pbi_subset_states_its_true_total() {
        let root = fresh_root("panels-backlog-capped");
        let mut jsonl = String::new();
        for i in 0..25 {
            jsonl.push_str(&format!(
                "{{\"kind\":\"pbi\",\"id\":\"PBI-{i}\",\"title\":\"Backlog item {i}\",\"status\":\"proposed\",\"feature\":\"demo\"}}\n"
            ));
        }
        write(&root, ".bee/backlog.jsonl", &jsonl);

        let st = build_state();
        let project = register(&st, &root, "panels-backlog-capped");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("proposed: 25"), "the status chip count must cover every PBI, capped or not: {body}");
        assert!(
            body.contains("Showing 20 of 25 backlog items."),
            "the capped PBI subset must state its true total: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 / bbp-11 (edge, split — see the split rationale above):
    /// no backlog. Both the PBI list and the finding list render their own
    /// honest empty state, no bare `0`.
    #[tokio::test]
    async fn absent_backlog_renders_honest_empty_states() {
        let root = fresh_root("panels-empty-backlog");
        write(&root, "README.md", "# hi");
        // A present-but-empty `.bee/` (D3) — no `.bee/` at all would 404
        // instead of rendering the honest-empty-state panel under test.
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let st = build_state();
        let project = register(&st, &root, "panels-empty-backlog");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("No backlog items yet."), "{body}");
        assert!(body.contains("No findings yet."), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 / bbp-11 (edge, split — see above): no sessions.
    #[tokio::test]
    async fn absent_sessions_renders_honest_empty_state() {
        let root = fresh_root("panels-empty-sessions");
        write(&root, "README.md", "# hi");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let st = build_state();
        let project = register(&st, &root, "panels-empty-sessions");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("No sessions recorded."), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// bee-cockpit-6 / bbp-11 (edge, split — see above): no lane records and
    /// no active feature. Replaces the split-out "No lanes running." /
    /// "No worktree workspaces yet." assertions — `bee_lanes_panel` no
    /// longer exists (bbp-11); the equivalent honest-empty-state guarantee
    /// now belongs to `bee_phase_board_section`
    /// (`empty_store_renders_honest_empty_phase_board`, above). The
    /// worktree-workspace half of the old assertion has no replacement in
    /// this cell (see `phase_board_places_lane_feature_at_its_recorded_phase`).
    #[tokio::test]
    async fn absent_lanes_renders_honest_empty_phase_board() {
        let root = fresh_root("panels-empty-lanes");
        write(&root, "README.md", "# hi");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let st = build_state();
        let project = register(&st, &root, "panels-empty-lanes");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-phase-board-count=\"0\""), "{body}");
        assert!(body.contains("No features are tracked by phase right now."), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    // ── bbp-14: the backlog & review panel ─────────────────────────────────

    /// (happy) Each PBI's own title now renders alongside its status, not
    /// only the per-status count — a manager reads WHAT is proposed or in
    /// flight, not only how many.
    #[tokio::test]
    async fn backlog_panel_lists_each_pbi_title_under_its_status() {
        let root = fresh_root("panels-backlog-titles");
        write(
            &root,
            ".bee/backlog.jsonl",
            "{\"kind\":\"pbi\",\"id\":\"PBI-1\",\"title\":\"Add search\",\"status\":\"in-flight\",\"feature\":\"demo\"}\n\
             {\"kind\":\"pbi\",\"id\":\"PBI-2\",\"title\":\"Add filter\",\"status\":\"done\",\"feature\":\"demo\"}\n",
        );

        let st = build_state();
        let project = register(&st, &root, "panels-backlog-titles");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Add search"), "PBI-1's title must render: {body}");
        assert!(body.contains("Add filter"), "PBI-2's title must render: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (untrusted) A PBI title carrying `< > & "` must render as text, never
    /// as markup — the same guarantee `esc()` already gives every other
    /// free-text field on the board, exercised here at the one new site
    /// this cell adds (bare PBI titles were never rendered before bbp-14).
    #[tokio::test]
    async fn untrusted_pbi_title_renders_as_text() {
        let root = fresh_root("panels-backlog-untrusted-title");
        write(
            &root,
            ".bee/backlog.jsonl",
            "{\"kind\":\"pbi\",\"id\":\"PBI-1\",\"title\":\"Fix <script>alert(1)</script> & \\\"quote\\\" it.\",\"status\":\"proposed\",\"feature\":\"demo\"}\n",
        );

        let st = build_state();
        let project = register(&st, &root, "panels-backlog-untrusted-title");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("Fix &lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quote&quot; it."),
            "escaped title missing: {body}"
        );
        assert!(!body.contains("<script>alert(1)</script>"), "raw script tag leaked: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (security, D9) A PBI title with an absolute path embedded mid-
    /// sentence must be scrubbed before it ever reaches the body — the same
    /// mid-sentence scrub already proven for `next_action` and a lane's
    /// `next_action`, exercised here at the new PBI-title render site.
    #[tokio::test]
    async fn backlog_pbi_title_embedded_absolute_path_does_not_leak() {
        let root = fresh_root("panels-backlog-title-scrub");
        let secret = root.join("src").join("secret.rs").to_string_lossy().into_owned();
        let secret_escaped = secret.replace('\\', "\\\\");
        write(
            &root,
            ".bee/backlog.jsonl",
            &format!(
                "{{\"kind\":\"pbi\",\"id\":\"PBI-1\",\"title\":\"Fix {path} before shipping.\",\"status\":\"proposed\",\"feature\":\"demo\"}}\n",
                path = secret_escaped,
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "panels-backlog-title-scrub");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(!body.contains(&secret), "the backlog panel leaked an absolute PBI-title path: {body}");
        assert!(body.contains("src/secret.rs"), "the reduced relative path should still read: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) The review queue states unreviewed / in review / settled
    /// counts from the candidates × sessions join, with the open-P1 count
    /// called out first as the sharpest number on the panel (D6).
    /// Independent review is worded as owner-invoked throughout (D7).
    #[tokio::test]
    async fn review_queue_states_counts_by_state_with_open_p1_called_out() {
        let root = fresh_root("panels-review-happy");
        write(
            &root,
            ".bee/review-candidates.jsonl",
            "{\"id\":\"c1\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"demo\",\"head\":\"h1\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-a\"]}\n\
             {\"id\":\"c2\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"demo\",\"head\":\"h2\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-b\"]}\n\
             {\"id\":\"c3\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"demo\",\"head\":\"h3\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-c\"]}\n",
        );
        write(
            &root,
            ".bee/reviews/r1.json",
            r#"{"id":"r1","included":[{"type":"cell","id":"cell-a"}],"findings":[],"decision":{"status":"approved"}}"#,
        );
        write(
            &root,
            ".bee/reviews/r2.json",
            r#"{"id":"r2","included":[{"type":"cell","id":"cell-b"}],"findings":[{"id":"f1","severity":"P1","title":"x"}],"decision":{"status":"pending"}}"#,
        );

        let st = build_state();
        let project = register(&st, &root, "panels-review-happy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Unreviewed: 1"), "{body}");
        assert!(body.contains("In review: 1"), "{body}");
        assert!(body.contains("Settled: 1"), "{body}");
        assert!(body.contains("1 open P1 finding"), "the open P1 count must be called out: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A store with no review candidates and no review sessions
    /// renders an honest UNKNOWN review state — never `0/0/0`. Zero
    /// unreviewed and "we have never looked" are different facts; the panel
    /// must not collapse them (the same instinct as D5's honest-empty-state
    /// rule elsewhere on the board).
    #[tokio::test]
    async fn review_queue_with_no_candidates_and_no_sessions_renders_unknown_not_zeros() {
        let root = fresh_root("panels-review-unknown");
        write(&root, "README.md", "# hi");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();

        let st = build_state();
        let project = register(&st, &root, "panels-review-unknown");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Review state unknown"), "{body}");
        assert!(!body.contains("Unreviewed: 0"), "an unknown review state must not render as a clean zero: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A store WITH review candidates but no `.bee/reviews/` session
    /// at all renders every candidate as `Unreviewed` — a real, computed
    /// count, distinct from the unknown state above (which has no
    /// candidates to count in the first place).
    #[tokio::test]
    async fn review_queue_with_candidates_but_no_sessions_renders_all_unreviewed() {
        let root = fresh_root("panels-review-all-unreviewed");
        write(
            &root,
            ".bee/review-candidates.jsonl",
            "{\"id\":\"c1\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"demo\",\"head\":\"h1\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-a\"]}\n\
             {\"id\":\"c2\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"demo\",\"head\":\"h2\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-b\"]}\n",
        );

        let st = build_state();
        let project = register(&st, &root, "panels-review-all-unreviewed");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(!body.contains("Review state unknown"), "candidates exist — this is a real count, not unknown: {body}");
        assert!(body.contains("Unreviewed: 2"), "{body}");
        assert!(body.contains("In review: 0"), "{body}");
        assert!(body.contains("Settled: 0"), "{body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (D7) Independent review reads as owner-invoked wherever the review
    /// queue names it, never as a stage the board implies runs on its own —
    /// checked against the panel's own body text rather than against a
    /// single fixed string, so a future wording change cannot silently
    /// reintroduce "pending" language.
    #[tokio::test]
    async fn review_queue_never_words_review_as_automatic_pending_work() {
        let root = fresh_root("panels-review-d7-wording");
        write(
            &root,
            ".bee/review-candidates.jsonl",
            "{\"id\":\"c1\",\"type\":\"candidate\",\"date\":\"2026-08-01T00:00:00.000Z\",\"feature\":\"demo\",\"head\":\"h1\",\"mode\":\"standard\",\"baseline\":null,\"cells\":[\"cell-a\"]}\n",
        );

        let st = build_state();
        let project = register(&st, &root, "panels-review-d7-wording");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("invoked by the owner"), "the review queue must read as owner-invoked: {body}");

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
        write(&root, ".bee/lanes/demo.json", &lane_json("demo", "swarming", "standard", "run tests", None, None));
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

    /// `reachable_links` / bbp-11: rendering the detail pages without
    /// linking to them does not satisfy this cell — the board body must
    /// actually carry both kinds of link. Re-expressed against the new
    /// markup: per-cell links no longer come from the by-phase board itself
    /// (D3 — a phase card links only to its feature's detail page, never a
    /// cell's), so the cell-detail link comes from the Running Now section
    /// instead, exactly as it always has for a live worker.
    #[tokio::test]
    async fn board_body_links_to_feature_and_running_cell_detail_routes() {
        let root = fresh_root("board-links");
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json(
                "link-cell",
                "link-feature",
                "claimed",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );
        write(&root, ".bee/lanes/link-feature.json", &lane_json("link-feature", "swarming", "standard", "keep going", None, None));
        write(
            &root,
            ".bee/state.json",
            r#"{"phase":"exploring","feature":"link-feature","mode":"standard","workers":[{"nickname":"w1","cell":"link-cell","tier":"generation","status":"running"}]}"#,
        );
        write(
            &root,
            ".bee/sessions/w1.json",
            &session_json("w1", &rfc3339_minutes_ago(1), "/home/x/t.jsonl", "main", "startup"),
        );

        let st = build_state();
        let project = register(&st, &root, "board-links");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/cell/link-cell\"", project.id)),
            "the running-now section must link a live worker's cell to its detail page: {body}"
        );
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/feature/link-feature\"", project.id)),
            "the phase board must link its feature to its detail page: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) The board's Finished section groups finished features — one
    /// compact line per feature, not one card per cell — and states the
    /// true total number of finished cells, matching `data-finished-cells`.
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

        assert!(body.contains("data-finished-features=\"1\""), "{body}");
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

        assert!(body.contains("data-finished-features=\"25\""), "{body}");
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

        assert!(body.contains("data-finished-features=\"2\" data-finished-cells=\"3\""), "{body}");
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

    /// (happy, bbp-11) A feature that is both lane-tracked and fully shipped
    /// renders once — in the Finished section — never also as an in-flight
    /// phase card. This is the overlap `board_renders_finished_work_in_exactly_one_place`
    /// (above) does not cover: that fixture's feature carries no lane at
    /// all, so it never reaches `phase_board` in the first place. This one
    /// exercises the dedup `bee_phase_board_section` performs against
    /// `shipped` explicitly.
    #[tokio::test]
    async fn lane_tracked_feature_that_has_shipped_renders_only_once_in_finished() {
        let root = fresh_root("finished-and-laned");
        write(
            &root,
            ".bee/lanes/shipped-and-laned.json",
            &lane_json("shipped-and-laned", "compound", "standard", "capture the moment", None, None),
        );
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json(
                "f1",
                "shipped-and-laned",
                "capped",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "finished-and-laned");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-phase-board-count=\"0\""), "{body}");
        assert!(body.contains("data-finished-features=\"1\""), "{body}");
        assert!(
            !body.contains("class=\"fg-card bee-cell bee-phase-card\""),
            "a shipped feature must never also render as an in-flight phase card: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // --- bbp-11: the by-phase board (D5, replacing the four cell buckets) ---

    /// (happy) Several features, each on their own lane at their own phase,
    /// render as phase cards linking to their own feature detail page.
    #[tokio::test]
    async fn phase_board_places_several_features_on_their_own_phase() {
        let root = fresh_root("phase-board-several");
        write(&root, ".bee/lanes/alpha.json", &lane_json("alpha", "shaping", "standard", "shape it", None, None));
        write(&root, ".bee/lanes/beta.json", &lane_json("beta", "swarming", "standard", "swarm it", None, None));

        let st = build_state();
        let project = register(&st, &root, "phase-board-several");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-phase-board-count=\"2\""), "{body}");
        assert!(body.contains("data-phase-col=\"shaping\""), "{body}");
        assert!(body.contains("data-phase-col=\"swarming\""), "{body}");
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/feature/alpha\"", project.id)),
            "{body}"
        );
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/feature/beta\"", project.id)),
            "{body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) The globally active feature (`state.feature`) that also
    /// carries its own lane record still places exactly one phase card —
    /// `compute_phase_board`'s union rule (bbp-10) dedupes by feature name.
    #[tokio::test]
    async fn active_feature_with_its_own_lane_appears_as_one_phase_card() {
        let root = fresh_root("phase-board-active-once");
        write(&root, ".bee/state.json", r#"{"phase":"swarming","feature":"active-feature","mode":"standard"}"#);
        write(
            &root,
            ".bee/lanes/active-feature.json",
            &lane_json("active-feature", "swarming", "standard", "keep going", None, None),
        );

        let st = build_state();
        let project = register(&st, &root, "phase-board-active-once");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-phase-board-count=\"1\""), "{body}");
        assert_eq!(
            body.matches("class=\"fg-card bee-cell bee-phase-card\"").count(),
            1,
            "the active feature's own lane record must not double its phase card: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A repo with no `.bee/lanes/` directory at all still places its
    /// one active feature honestly, not an empty board — this repo's own
    /// store is exactly this fixture shape
    /// (`docs/history/bee-board-pm/plan.md` Discovery).
    #[tokio::test]
    async fn repo_with_no_lane_records_places_its_one_active_feature() {
        let root = fresh_root("phase-board-no-lanes-dir");
        write(&root, ".bee/state.json", r#"{"phase":"exploring","feature":"lone-active-feature","mode":"standard"}"#);
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json("c-open", "lone-active-feature", "open", &[], "w1", "x", "y"),
        );
        // Deliberately no `.bee/lanes/` directory at all.

        let st = build_state();
        let project = register(&st, &root, "phase-board-no-lanes-dir");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-phase-board-count=\"1\""), "{body}");
        assert!(
            body.contains(&format!("href=\"/p/{}/_bee/feature/lone-active-feature\"", project.id)),
            "the one active feature must render honestly even with no lane records at all: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A feature placed on the board with zero live cells renders an
    /// honest progress line, never a `0/0` division artifact.
    #[tokio::test]
    async fn phase_card_with_no_cells_renders_honest_progress() {
        let root = fresh_root("phase-board-no-cells");
        write(
            &root,
            ".bee/lanes/empty-feature.json",
            &lane_json("empty-feature", "exploring", "standard", "get started", None, None),
        );

        let st = build_state();
        let project = register(&st, &root, "phase-board-no-cells");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("No live cells recorded for this feature yet."),
            "expected an honest progress line: {body}"
        );
        assert!(!body.contains("0/0"), "a division artifact leaked in: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge, D8) A feature whose cells are all dropped shows no completed
    /// work on its phase card — dropped cells count toward no denominator
    /// and no total.
    #[tokio::test]
    async fn phase_card_with_all_dropped_cells_shows_no_completed_work() {
        let root = fresh_root("phase-board-all-dropped");
        write(
            &root,
            ".bee/lanes/dropped-feature.json",
            &lane_json("dropped-feature", "swarming", "standard", "n/a", None, None),
        );
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json("d1", "dropped-feature", "dropped", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "phase-board-all-dropped");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("No live cells recorded for this feature yet."),
            "an all-dropped feature must show no completed work, honestly: {body}"
        );
        assert!(!body.contains("0/0"), "a division artifact leaked in: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (untrusted) A feature name and a lane's `next_action` containing
    /// `< > & "` render as text on the phase board, never breaking the
    /// surrounding markup.
    #[tokio::test]
    async fn phase_card_untrusted_feature_name_and_next_action_render_as_text() {
        let root = fresh_root("phase-board-untrusted");
        let untrusted_feature = "<b>bold</b> feature";
        let lane = format!(
            r#"{{"feature": {feature}, "phase": "swarming", "mode": "standard", "next_action": "Fix <script>alert(1)</script> & \"quote\" it."}}"#,
            feature = serde_json::to_string(untrusted_feature).unwrap(),
        );
        write(&root, ".bee/lanes/untrusted.json", &lane);

        let st = build_state();
        let project = register(&st, &root, "phase-board-untrusted");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("&lt;b&gt;bold&lt;/b&gt; feature"),
            "untrusted feature name must render escaped, as text: {body}"
        );
        assert!(!body.contains("<b>bold</b>"), "the raw feature-name tag must not survive: {body}");
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

    /// (security) An absolute path embedded mid-sentence in a lane's
    /// `next_action` must not leak into the phase board — the same
    /// mid-sentence scrub
    /// `board_embedded_absolute_path_in_rationale_and_next_action_does_not_leak`
    /// already proves for `state.json`'s fields, exercised here through a
    /// lane record instead.
    #[tokio::test]
    async fn phase_card_embedded_absolute_path_in_next_action_does_not_leak() {
        let root = fresh_root("phase-board-security-scrub");
        let secret = root.join("src").join("secret.rs").to_string_lossy().into_owned();
        let secret_escaped = secret.replace('\\', "\\\\");
        let lane = format!(
            r#"{{"feature": "scrub-feature", "phase": "swarming", "mode": "standard", "next_action": "Read {path} then continue."}}"#,
            path = secret_escaped,
        );
        write(&root, ".bee/lanes/scrub-feature.json", &lane);

        let st = build_state();
        let project = register(&st, &root, "phase-board-security-scrub");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            !body.contains(&secret),
            "the phase board leaked an absolute path embedded in a lane's next_action: {body}"
        );
        assert!(body.contains("src/secret.rs"), "the reduced relative path should still read: {body}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only) The by-phase board reads the same `.bee/lanes/*.json` and
    /// `.bee/cells/*.json` files as every other bee-cockpit route (D4) — the
    /// fixture tree must stay byte-identical before and after the request.
    #[tokio::test]
    async fn phase_board_read_never_writes_the_fixtures_bee_tree() {
        let root = fresh_root("phase-board-read-only");
        write(&root, "README.md", "# hi");
        write(
            &root,
            ".bee/lanes/demo.json",
            &lane_json("demo", "swarming", "standard", "run tests", None, None),
        );
        write(&root, ".bee/cells/a.json", &cell_json("c-open", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "phase-board-read-only");
        let before = snapshot_tree(&root);

        let _ = get(router(st), &format!("/p/{}/_bee", project.id)).await;

        let after = snapshot_tree(&root);
        assert_eq!(before, after, ".bee/ tree changed after a request");

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

    /// (read-only, bbp-13) The fixture `.bee/` tree is byte-identical after
    /// a board request whose fixture CONTAINS a populated
    /// `.bee/review-candidates.jsonl` — the pre-existing read-only tests
    /// each use a fixture with no candidates file, so they would pass
    /// green without the review-candidates reader ever running. This one
    /// exercises it: the reader must open the file, parse it and join it
    /// (unreviewed, since no `.bee/reviews/` session exists here) without
    /// ever writing back to it.
    #[tokio::test]
    async fn board_reading_is_read_only_with_a_populated_review_candidates_file_present() {
        let root = fresh_root("review-candidates-read-only");
        write(
            &root,
            ".bee/review-candidates.jsonl",
            r#"{"id":"c1","type":"candidate","date":"2026-08-06T00:00:00.000Z","feature":"review-candidates-read-only","head":"abc123","mode":"high-risk","baseline":null,"cells":["c-open"]}"#,
        );
        write(
            &root,
            ".bee/cells/c-open.json",
            &timed_cell_json("c-open", "review-candidates-read-only", "open", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "review-candidates-read-only");
        let before = snapshot_tree(&root);

        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_string(resp).await;
        assert!(
            body.contains("high-risk"),
            "the unreviewed high-risk candidate should surface as an attention item: body missing expected text"
        );

        let after = snapshot_tree(&root);
        assert_eq!(
            before, after,
            ".bee/ tree changed after a request whose fixture carries a populated review-candidates.jsonl"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only, bbp-13) The fixture `.bee/` tree is byte-identical after
    /// a board request whose fixture CONTAINS a populated
    /// `.bee/reviews/<id>.json` session — the pre-existing read-only tests
    /// each use a fixture with no `.bee/reviews/` directory at all, so they
    /// would pass green without the review-session reader ever running.
    /// This one exercises it: the reader must open the directory, parse
    /// the session and count its open P1 finding without ever writing back
    /// to it.
    #[tokio::test]
    async fn board_reading_is_read_only_with_a_populated_review_session_present() {
        let root = fresh_root("review-session-read-only");
        write(
            &root,
            ".bee/reviews/r1.json",
            r#"{"id":"r1","included":[],"findings":[{"id":"f1","severity":"P1","title":"unresolved"}],"decision":{"status":"pending"}}"#,
        );
        write(
            &root,
            ".bee/cells/c-open.json",
            &timed_cell_json("c-open", "review-session-read-only", "open", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "review-session-read-only");
        let before = snapshot_tree(&root);

        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_string(resp).await;
        assert!(
            body.contains("P1"),
            "the open P1 finding should surface as an attention item: body missing expected text"
        );

        let after = snapshot_tree(&root);
        assert_eq!(
            before, after,
            ".bee/ tree changed after a request whose fixture carries a populated review session"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only, bbp-13) The fixture `.bee/` tree is byte-identical after
    /// a board request whose fixture CONTAINS a populated
    /// `.bee/capture-queue.jsonl` — the pre-existing read-only tests each
    /// use a fixture with no capture-queue file, so they would pass green
    /// without the capture-queue reader ever running. This one exercises
    /// it: the reader must open the file, parse it and net the waiting
    /// stubs against any flush without ever writing back to it.
    #[tokio::test]
    async fn board_reading_is_read_only_with_a_populated_capture_queue_present() {
        let root = fresh_root("capture-queue-read-only");
        write(
            &root,
            ".bee/capture-queue.jsonl",
            r#"{"kind":"stub","id":"s1","at":"2026-08-06T00:00:00.000Z","outcome":"a note waiting to be written"}"#,
        );
        write(
            &root,
            ".bee/cells/c-open.json",
            &timed_cell_json("c-open", "capture-queue-read-only", "open", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "capture-queue-read-only");
        let before = snapshot_tree(&root);

        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = body_string(resp).await;
        assert!(
            body.contains("knowledge-debt"),
            "the waiting capture stub should surface as a knowledge-debt attention item: body missing expected text"
        );

        let after = snapshot_tree(&root);
        assert_eq!(
            before, after,
            ".bee/ tree changed after a request whose fixture carries a populated capture-queue.jsonl"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (read-only, bbp-15/bbp-16) The fixture `.bee/` tree is byte-identical
    /// after a board request whose fixture CONTAINS a populated
    /// `.bee/reservations.json` — the pre-existing read-only tests each use
    /// a fixture with no reservations file, so they would pass green
    /// without the reservations reader ever running. This one exercises
    /// it: the reader must open the file and parse its array without ever
    /// writing back to it. bbp-16 adds the process-health panel that
    /// renders `reservations` (see `process_health_panel_renders_lock_contention_tier_mix_and_gate_bypass`
    /// for the fuller happy-path assertions); this test now also proves the
    /// unreleased lock actually surfaces here, not only that reading it is
    /// safe.
    #[tokio::test]
    async fn board_reading_is_read_only_with_a_populated_reservations_file_present() {
        let root = fresh_root("reservations-read-only");
        write(
            &root,
            ".bee/reservations.json",
            r#"{"reservations": [
                {"agent": "w1", "cell": "c-open", "path": "src/lib.rs", "kind": "lease", "session": "s1", "reserved_at": "2026-08-06T00:00:00.000Z", "released_at": null}
            ]}"#,
        );
        write(
            &root,
            ".bee/cells/c-open.json",
            &timed_cell_json("c-open", "reservations-read-only", "open", &[], "w1", "x", "y"),
        );

        let st = build_state();
        let project = register(&st, &root, "reservations-read-only");
        let before = snapshot_tree(&root);

        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("src/lib.rs"),
            "the process-health panel must render the unreleased reservation: {body}"
        );

        let after = snapshot_tree(&root);
        assert_eq!(
            before, after,
            ".bee/ tree changed after a request whose fixture carries a populated reservations.json"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy, bbp-16) Process health renders all three of bbp-15's
    /// derivations at once: an unreleased reservation as file-lock
    /// contention, the tier-mix chips, and the recorded `gate_bypass`
    /// setting worded exactly as `compute_attention_items`' own gate-bypass
    /// rule words it (`mdview_core::bee`) — `Gate bypass recorded as
    /// "{level}"` — so the panel and the attention item never drift into
    /// disagreeing phrasing for the same fact.
    #[tokio::test]
    async fn process_health_panel_renders_lock_contention_tier_mix_and_gate_bypass() {
        let root = fresh_root("process-health-happy");
        write(&root, ".bee/config.json", r#"{"gate_bypass": "total"}"#);
        write(
            &root,
            ".bee/reservations.json",
            r#"{"reservations": [
                {"agent": "lastband", "cell": "kf-1", "path": "src/lib.rs", "kind": "lease", "session": "s1", "reserved_at": "2026-08-06T00:00:00.000Z", "released_at": null},
                {"agent": "other", "cell": "kf-2", "path": "src/old.rs", "kind": "lease", "session": "s2", "reserved_at": "2026-08-05T00:00:00.000Z", "released_at": "2026-08-05T01:00:00.000Z"}
            ]}"#,
        );
        write(&root, ".bee/cells/kf-1.json", &cell_json("kf-1", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "process-health-happy");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Process health"), "{body}");
        assert!(
            body.contains("src/lib.rs") && body.contains("lastband"),
            "the still-locked reservation must render: {body}"
        );
        assert!(
            !body.contains("src/old.rs"),
            "a released reservation is not contention and must not render: {body}"
        );
        assert!(body.contains("generation: 1"), "the tier-mix chip must render: {body}");
        assert!(
            body.contains("Gate bypass recorded as \"total\""),
            "the recorded bypass level must be worded exactly as the attention rule words it: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge, bbp-16) A store with `read_errors` (a malformed
    /// `.bee/lanes/*.json` file here) shows them on the page — a partly-
    /// unreadable store must be visible, never silently thinner. Both the
    /// attention panel's own summary line and the process-health panel's
    /// own detailed list carry this, since `bee_read_errors` (`views.rs`)
    /// now renders inside the process-health panel.
    #[tokio::test]
    async fn process_health_panel_shows_read_errors_from_a_malformed_store_file() {
        let root = fresh_root("process-health-read-errors");
        write(&root, ".bee/lanes/broken.json", "{ not valid json");
        write(&root, ".bee/cells/a.json", &cell_json("a", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "process-health-read-errors");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("Could not read"), "{body}");
        assert!(
            body.contains(".bee/lanes/broken.json"),
            "the specific unreadable file must be named: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (happy) bee-board-ux-2 / bbp-11: a finished feature's line in the
    /// Finished section still links to its feature detail page, and an
    /// in-flight feature's phase card links to its feature detail page too
    /// — both drill-downs the board exists to reach. Re-expressed against
    /// the new markup: the old fixture's "live" cell carried no lane and no
    /// active-feature record, so under bbp-11 it would never place on the
    /// board at all; a per-cell link from a bare board card is no longer a
    /// guarantee this cell makes (see `board_card_drops_file_list_but_cell_detail_page_keeps_it`
    /// and `board_body_links_to_feature_and_running_cell_detail_routes` for
    /// where cell-level links still live).
    #[tokio::test]
    async fn board_links_finished_feature_and_in_flight_feature_to_their_detail_pages() {
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
            ".bee/lanes/live-feature.json",
            &lane_json("live-feature", "swarming", "standard", "keep going", None, None),
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
            body.contains(&format!("href=\"/p/{}/_bee/feature/live-feature\"", project.id)),
            "an in-flight feature's phase card must link to its feature detail page: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (edge) A fixture with nothing finished renders an honest empty
    /// Finished section — not a zero presented as a real measurement.
    #[tokio::test]
    async fn board_done_section_renders_honest_empty_state_when_nothing_done() {
        let root = fresh_root("done-empty");
        write(&root, ".bee/cells/a.json", &cell_json("open-only", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "done-empty");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(body.contains("data-finished-features=\"0\""), "{body}");
        assert!(body.contains("Nothing finished yet."), "{body}");
        // honest empty state: no "N finished cell(s) total" note manufactured
        // from a zero.
        assert!(!body.contains("finished cell"), "{body}");
        // an honest empty state, not a collapsed empty list — no <details>
        // wrapper when there is nothing to show.
        assert!(
            !body.contains("bee-done-details"),
            "empty Finished section must not render as a collapsed empty list: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (regression, bbp-11) A card on the board no longer prints a cell's
    /// file list — that detail moved to the cell detail page, which still
    /// shows it. The fixture places its feature on the by-phase board (a
    /// lane record) so this actually exercises `bee_phase_card`, which has
    /// no `files` field on `BeeFeaturePhase` to print in the first place.
    #[tokio::test]
    async fn board_card_drops_file_list_but_cell_detail_page_keeps_it() {
        let root = fresh_root("board-no-files");
        write(
            &root,
            ".bee/lanes/demo.json",
            &lane_json("demo", "swarming", "standard", "keep going", None, None),
        );
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

    /// (regression, bbp-11) A feature known only through a lane record, with
    /// no live cells at all (routine in a real store — cells archive after a
    /// feature closes; `counter-teeth` in beehive's own store is exactly
    /// this shape), resolves its feature detail page rather than 404ing.
    /// Found by loading the rendered board and following a phase card's own
    /// link, not by the suite: before this fix, `bee_feature_detail`
    /// recognized a feature only through its live D7 buckets or `shipped`,
    /// so every phase card for a lane-only feature 404'd — the exact link
    /// bbp-11's `bee_phase_board_section` now renders on every phase card.
    #[tokio::test]
    async fn lane_only_feature_with_no_live_cells_resolves_its_detail_page() {
        let root = fresh_root("lane-only-feature-detail");
        write(
            &root,
            ".bee/lanes/archived-feature.json",
            &lane_json("archived-feature", "compounding-complete", "standard", "none", None, None),
        );
        // Deliberately no `.bee/cells/*.json` for this feature at all.

        let st = build_state();
        let project = register(&st, &root, "lane-only-feature-detail");
        let app = router(st);

        let board_resp = get(app.clone(), &format!("/p/{}/_bee", project.id)).await;
        let board_body = body_string(board_resp).await;
        assert!(
            board_body.contains(&format!(
                "href=\"/p/{}/_bee/feature/archived-feature\"",
                project.id
            )),
            "the phase board must place this lane-only feature: {board_body}"
        );

        let feature_resp = get(app, &format!("/p/{}/_bee/feature/archived-feature", project.id)).await;
        assert_eq!(
            feature_resp.status(),
            StatusCode::OK,
            "a feature the board just linked to must resolve, not 404"
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

    // ── bee-board-ux-3: "running now" — bbp-16 folds this into the
    // "Working on now" card's own "Running now" subsection
    // (`bee_running_workers_section`, `views.rs`); the assertions below are
    // otherwise unchanged, since that row-level markup itself did not
    // change, only its home. ──────────────────────────

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

    /// (happy, bbp-11) The presence of a worker naming a still-open cell
    /// must never move that cell's phase-card progress (D5's "phase
    /// membership is a pure function of the store" rule) — re-expressed
    /// against the new markup: `compute_phase_board`/`compute_feature_cell_counts`
    /// (`mdview_core::bee`) never take `running_workers` as an input, so the
    /// phase card for `state.feature` ("demo", the only feature this fixture
    /// declares) must keep reading "0/1 cell done" even though a live
    /// worker names its one open cell.
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
            body.contains("data-phase-board-count=\"1\""),
            "the active feature must still place on the phase board: {body}"
        );
        assert!(
            body.contains("0/1 cell done"),
            "an open cell must keep reading as not-done even though a live worker names it: {body}"
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
            "bbp-16 retired the standalone running-now section entirely — this class must never render again, from any state",
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
        // The Sessions panel legitimately still lists a stale session
        // (unrelated pre-existing behavior, now folded into
        // `bee_sessions_panel`); what must be absent is any worker card for
        // it in the working-now card's own "Running now" subsection — and,
        // per bbp-16, the standalone running-now section's class must never
        // render again at all.
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

    // --- bee-board-ux-4: each granted worktree, its own lifecycle record —
    // bbp-16 folds this section into `bee_sessions_panel`'s own "Worktrees"
    // subhead (`bee_worktrees_body`, `views.rs`); the row-level markup these
    // assertions check is otherwise unchanged, only its wrapper moved. ---

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

    /// (happy, bbp-11) Worktree cell files present in the fixture change
    /// NEITHER this project's phase-board cell counts NOR its shipped set —
    /// the regression that motivated this cell, re-expressed against the new
    /// markup: `compute_phase_board`/`compute_feature_cell_counts`
    /// (`mdview_core::bee`) only ever see this project's own
    /// `.bee/cells/*.json`, so a worktree's own capped cell can never move
    /// this project's phase card's progress, and its cell id never appears
    /// on this board at all (`BeeFeaturePhase` carries no cell ids). The
    /// worktree's own *feature name* legitimately still appears — in the
    /// Worktrees panel, naming which feature that worktree runs — this test
    /// asserts only that its cell never merges into this project's own
    /// counts.
    #[tokio::test]
    async fn worktree_cell_files_do_not_change_buckets_or_shipped_set() {
        let root = fresh_root("wt-no-cell-merge");
        write(&root, ".bee/cells/a.json", &cell_json("c-open", "open", &[], "w1"));
        write(&root, ".bee/lanes/demo.json", &lane_json("demo", "swarming", "standard", "keep going", None, None));

        let sibling = make_worktree_sibling("bee-board-ux-4-srv-wt-cells");
        write(&sibling, ".bee/state.json", r#"{"phase":"swarming","feature":"ghost-feature","mode":"standard"}"#);
        // A capped cell sitting only in the worktree's own store. If this
        // were ever merged into the main snapshot it would count toward this
        // project's phase-board progress and appear as an extra shipped
        // feature.
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

        assert!(body.contains("data-phase-board-count=\"1\""), "{body}");
        assert!(
            body.contains("0/1 cell done"),
            "a worktree's own capped cell must never count toward this project's phase card progress: {body}"
        );
        assert!(body.contains("data-finished-features=\"0\""), "{body}");
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
            "bbp-16 retired the standalone worktrees section — this class must never render again, from any state",
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

    // toa-3 (D7): `api_config_never_contains_the_token_value_before_or_after_generation`
    // (D10/P1) and `settings_page_reveals_the_token_in_full_once_then_masks_it`
    // (P2) are retired, not re-expressed — their whole subject was the
    // terminal token's generation and reveal-once rendering, and D1 removes
    // that token, its rotation route, and every render of it along with
    // `terminal_auth`. There is no value left for either test to assert
    // never leaks or never shows twice.

    /// D1/toa-3: the login route, the rotation route, and every token
    /// control the settings page used to render are gone — not disabled,
    /// not hidden, gone. The settings page has no terminal token section at
    /// all (the "honest empty" this cell's own must-haves name), and both
    /// routes answer with mdview's ordinary unrouted response now that
    /// nothing mounts them.
    #[tokio::test]
    async fn settings_page_carries_no_terminal_token_controls_and_the_routes_are_gone() {
        let dir = fresh_root("terminal-token-controls-removed");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let resp = get(app.clone(), "/settings").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        for needle in [
            "/settings/terminal/token",
            "/settings/terminal/login",
            "Terminal token",
            "Terminal sign-in",
            "Generate token",
            "Rotate token",
            "Sign in",
        ] {
            assert!(
                !body.contains(needle),
                "/settings still carries a terminal token control ({needle:?}): {body}"
            );
        }

        for uri in ["/settings/terminal/token", "/settings/terminal/login"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "{uri} must no longer be routed at all"
            );
        }

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
                "port=58312&enabled=on&supervisor_enabled=on&notify_enabled=on&unassigned_enabled=on",
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
        assert!(
            !saved.terminal.unassigned_enabled,
            "POST /api/config flipped the toa-4 (D9) Unassigned group switch"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// toa-1 (D1): the switches can be changed by any request that reaches
    /// `POST /api/terminal-config` — no cookie, no token. D10: the only
    /// admission fee left is a JSON body — this is the "happy" proof that a
    /// JSON POST sets every switch exactly as the old form submission did,
    /// including the redirect the settings page relies on.
    #[tokio::test]
    async fn a_json_post_sets_every_switch_with_no_cookie_and_no_token() {
        let dir = fresh_root("terminal-switches-open");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "enabled": true,
                            "supervisor_enabled": true,
                            "notify_enabled": true,
                            "unassigned_enabled": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status().is_redirection(),
            "the switch endpoint must be reachable with no cookie and no token, got {}",
            resp.status()
        );

        let saved = Config::load_from(&dir.join("config.toml"));
        assert!(saved.terminal.enabled);
        assert!(saved.terminal.supervisor_enabled);
        assert!(saved.terminal.notify_enabled);
        assert!(saved.terminal.unassigned_enabled);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D10, the security proof this cell exists for: a form-encoded POST —
    /// a CORS *simple* request, sendable cross-site with no preflight and
    /// no CORS layer on this server — must change no switch. The assertion
    /// is on the stored config, not the response status, per
    /// `docs/history/learnings/20260805-toothless-security-assertions.md`:
    /// a status-code-only check would stay green even if the extractor
    /// rejected the request but the handler had already run.
    #[tokio::test]
    async fn a_form_encoded_post_to_terminal_config_changes_no_switch() {
        let dir = fresh_root("terminal-config-form-refused");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let _resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("enabled=on&supervisor_enabled=on&notify_enabled=on"))
                    .unwrap(),
            )
            .await
            .unwrap();

        let saved = Config::load_from(&dir.join("config.toml"));
        assert!(
            !saved.terminal.enabled,
            "a form-encoded POST turned the terminal switch on"
        );
        assert!(
            !saved.terminal.supervisor_enabled,
            "a form-encoded POST turned the supervisor switch on"
        );
        assert!(
            !saved.terminal.notify_enabled,
            "a form-encoded POST turned the notify switch on"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D10, edge: a JSON body carrying a field the server does not
    /// recognize is handled without changing the switches or fields the
    /// request also carries — an unknown key is silently ignored (no
    /// `#[serde(deny_unknown_fields)]`), never a rejection.
    #[tokio::test]
    async fn a_json_post_with_an_unknown_field_leaves_the_recognized_fields_correct() {
        let dir = fresh_root("terminal-config-unknown-field");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "enabled": true,
                            "totally_unknown_field": "surprise",
                            "notify_chat_id": "12345"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status().is_redirection(),
            "an unknown field must not make the whole request fail, got {}",
            resp.status()
        );

        let saved = Config::load_from(&dir.join("config.toml"));
        assert!(saved.terminal.enabled, "the recognized switch was not set");
        assert!(
            !saved.terminal.supervisor_enabled,
            "an unrelated switch changed because of the unknown field"
        );
        assert!(
            !saved.terminal.notify_enabled,
            "an unrelated switch changed because of the unknown field"
        );
        assert_eq!(saved.terminal.notify_chat_id.as_deref(), Some("12345"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D10/D4, edge: an empty `notify_telegram_token` field leaves whatever
    /// credential is already on disk alone — a blank JSON string must mean
    /// the same "leave it alone" the blank form field always meant, never
    /// "clear it".
    #[tokio::test]
    async fn a_json_post_with_an_empty_credential_field_leaves_the_stored_credential_alone() {
        let dir = fresh_root("terminal-config-empty-cred");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());
        let cred_path = mdview_core::config::notify_credential_path_override(Some(&dir));

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "notify_telegram_token": "keep-me" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(first.status().is_redirection());
        assert_eq!(
            mdview_core::config::load_notify_credential(&cred_path).as_deref(),
            Some("keep-me")
        );

        let second = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({ "notify_telegram_token": "" }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(second.status().is_redirection());
        assert_eq!(
            mdview_core::config::load_notify_credential(&cred_path).as_deref(),
            Some("keep-me"),
            "an empty credential field must leave the stored credential alone, not clear it"
        );

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

        assert!(!st.terminal_background.supervisor_running());
        assert!(!st.terminal_background.notify_running());

        let on_req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "enabled": true,
                    "supervisor_enabled": true,
                    "notify_enabled": true
                })
                .to_string(),
            ))
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
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({ "enabled": true }).to_string()))
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

    /// toa-1 (D1): the destination and credential the settings page's
    /// notification section writes are reachable through
    /// `update_terminal_config` with no cookie and no token, same as the
    /// switches above.
    #[tokio::test]
    async fn notify_destination_and_credential_are_reachable_with_no_cookie_and_no_token() {
        let dir = fresh_root("terminal-notify-open");
        let st = build_state_with_dir(&dir);
        let app = router(st);
        let cred_path = mdview_core::config::notify_credential_path_override(Some(&dir));

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "notify_chat_id": "12345",
                            "notify_telegram_token": "secret-bot-token"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status().is_redirection(),
            "the notify fields must be reachable with no cookie and no token, got {}",
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
    /// after it is saved, checked against the real submitted secret value
    /// rather than a hardcoded guess (a leak assertion must be written
    /// against the value that would leak —
    /// `docs/history/learnings/20260805-toothless-security-assertions.md`).
    #[tokio::test]
    async fn api_config_never_contains_the_notify_credential() {
        let dir = fresh_root("terminal-notify-cred-not-in-api-config");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());

        let secret = "unique-telegram-secret-98765";
        let req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "notify_chat_id": "555", "notify_telegram_token": secret })
                    .to_string(),
            ))
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

    /// The credential has no reveal path at all — every render, including
    /// `/settings` immediately after the save that set it, carries at most
    /// its last four characters.
    #[tokio::test]
    async fn settings_page_never_reveals_the_notify_credential_in_full() {
        let dir = fresh_root("terminal-notify-cred-never-full");
        let st = build_state_with_dir(&dir);
        let app = router(st.clone());

        let secret = "never-shown-in-full-abcd";
        let req = Request::builder()
            .method("POST")
            .uri("/api/terminal-config")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "notify_chat_id": "555", "notify_telegram_token": secret })
                    .to_string(),
            ))
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
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "notify_telegram_token": secret }).to_string(),
            ))
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
        assert_eq!(json["terminal"]["unassigned_enabled"], serde_json::json!(false));

        std::fs::remove_dir_all(&dir).ok();
    }

    // toa-3 (D7): `rotate_token` is retired along with the login and
    // rotation routes it drove — there is no credential left to obtain and
    // nothing left that checks one. The `*_req` helpers below keep their
    // `cookie: Option<&str>` shape (every call site now passes `None`)
    // rather than a further signature rewrite; D1's own coverage
    // (`every_terminal_route_answers_with_no_cookie_and_no_token_present`)
    // is the proof that carrying a cookie was never required.

    /// A GET request to `/p/{id}/_terminal`, optionally carrying a session
    /// cookie value — every caller now passes `None` (D1: no route checks
    /// one), the parameter stays only so a future regression could still be
    /// probed by passing `Some`.
    fn terminal_req(id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal"))
            .method("GET");
        if let Some(c) = cookie {
            b = b.header(header::COOKIE, c.to_string());
        }
        b.body(Body::empty()).unwrap()
    }

    /// terminal-pane-scope D4: a GET to one pane's own terminal page —
    /// `/p/{id}/_terminal/pane/{pane_id}` — the pane-scoped sibling of
    /// `terminal_req` above.
    fn terminal_pane_req(id: &str, pane_id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_terminal/pane/{pane_id}"))
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

    /// toa-4 (D9): writes `dir/config.toml` with the Unassigned group's own
    /// switch on, leaving whatever `terminal.enabled` already is untouched.
    /// The two switches are ANDed (`unassigned_group_enabled`'s doc) — a
    /// test that wants the group's routes to actually run must call
    /// `enable_terminal` too, in either order.
    fn enable_unassigned_group(dir: &Path) {
        let mut cfg = Config::load_from(&dir.join("config.toml"));
        cfg.terminal.unassigned_enabled = true;
        cfg.save_to(&dir.join("config.toml")).unwrap();
    }

    /// D11: a `GET` carrying switch values in its query string changes no
    /// switch. Replaces `api_terminal_config_wrong_method_is_byte_identical_to_unrouted`
    /// (D5/D12 struck the byte-identical-to-unrouted comparison, and D11
    /// means this is no longer about hiding the route — `Form` reads the
    /// query string on a `GET` in axum 0.7.9, `form.rs:85`/`raw_form.rs:41-42`
    /// — so with the method gate gone, a route still mounted `any(...)` could
    /// be driven by a plain navigation or an `<img src>`. `post(...)` alone
    /// is what closes that, proven here.
    #[tokio::test]
    async fn a_get_carrying_switch_values_in_its_query_changes_no_switch() {
        let dir = fresh_root("terminal-config-get-query");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/terminal-config?enabled=on&supervisor_enabled=on&notify_enabled=on")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "GET must never reach the switch handler at all"
        );

        let saved = Config::load_from(&dir.join("config.toml"));
        assert!(!saved.terminal.enabled, "a GET must never flip the enabled switch");
        assert!(
            !saved.terminal.supervisor_enabled,
            "a GET must never flip the supervisor switch"
        );
        assert!(!saved.terminal.notify_enabled, "a GET must never flip the notify switch");

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

        let resp = app
            .oneshot(terminal_req("no-such-project", None))
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

        let resp = app
            .oneshot(terminal_req(&project.id, None))
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
    ///
    /// terminal-pane-scope D4: `terminal_page` now renders exactly one
    /// pane's full card, so membership here is read off the pane strip's own
    /// per-pane hrefs (`/p/:id/_terminal/pane/:pane_id`) rather than off an
    /// agent name appearing in a card — the strip lists every pane the D2
    /// boundary accepted, whichever one is the page's own selected card.
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

        let resp_a = app
            .clone()
            .oneshot(terminal_req(&project_a.id, None))
            .await
            .unwrap();
        assert_eq!(resp_a.status(), StatusCode::OK);
        let body_a = body_string(resp_a).await;
        let strip_href = |project_id: &str, pane_id: &str| format!("/p/{project_id}/_terminal/pane/{pane_id}");
        assert!(
            body_a.contains(&strip_href(&project_a.id, &at_root.pane_id)),
            "root pane missing from the strip: {body_a}"
        );
        assert!(
            body_a.contains(&strip_href(&project_a.id, &below_agent.pane_id)),
            "below-root pane missing from the strip: {body_a}"
        );
        assert!(
            !body_a.contains(&strip_href(&project_a.id, &above.pane_id)),
            "above-root pane leaked into the strip: {body_a}"
        );
        assert!(
            !body_a.contains(&strip_href(&project_a.id, &via_symlink.pane_id)),
            "symlink-escape pane leaked into the strip: {body_a}"
        );
        assert!(
            !body_a.contains(&strip_href(&project_a.id, &other_project.pane_id)),
            "project-b's pane leaked into project-a's strip: {body_a}"
        );

        let resp_b = app
            .oneshot(terminal_req(&project_b.id, None))
            .await
            .unwrap();
        let body_b = body_string(resp_b).await;
        assert!(
            body_b.contains(&strip_href(&project_b.id, &other_project.pane_id)),
            "project-b's own pane missing from its own strip: {body_b}"
        );
        assert!(
            !body_b.contains(&strip_href(&project_b.id, &at_root.pane_id)),
            "project-a's pane leaked into project-b's strip: {body_b}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D12: with the D7 `terminal.enabled` switch off, `terminal_page`
    /// answers with the ordinary not-found page and `terminal_screen`
    /// answers with a reasoned JSON 404 — no cookie, no token, nothing but
    /// the switch decides this.
    #[tokio::test]
    async fn terminal_family_disabled_answers_with_the_disabled_shapes() {
        let dir = fresh_root("terminal-family-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("terminal-family-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "family-disabled");
        let app = router(st);

        let page = app.clone().oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(page.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            page.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8",
            "the disabled terminal page must answer the ordinary not-found page, not an empty body"
        );
        let page_body = body_string(page).await;
        assert!(!page_body.is_empty(), "a disabled terminal must never answer with nothing");

        let screen = app
            .oneshot(terminal_screen_req(&project.id, "w1:p1", None))
            .await
            .unwrap();
        assert_eq!(screen.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            screen.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled screen endpoint must answer JSON, not an empty body"
        );
        let screen_body = body_string(screen).await;
        assert!(
            screen_body.contains("disabled"),
            "the disabled screen endpoint must name the reason: {screen_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// D1's core happy-path truth: with the terminal enabled and no cookie
    /// or token presented at all, every route in the family reaches its own
    /// real logic — never the old auth refusal, whose signature was an
    /// empty body regardless of status. A pane id this test never seeded
    /// (`FakeHerdr::new()`'s default snapshot has none) still answers with a
    /// real, non-empty "pane not found" — proof the request got *past* where
    /// the auth extractors used to sit, not that every route happens to
    /// answer `200`.
    #[tokio::test]
    async fn every_terminal_route_answers_with_no_cookie_and_no_token_present() {
        let dir = fresh_root("terminal-no-cookie-happy-path");
        enable_terminal(&dir);
        // toa-4 (D9): the Unassigned assertions below need their own switch
        // too, or they exercise the disabled response rather than the real,
        // no-auth logic this test is proving.
        enable_unassigned_group(&dir);
        let root = fresh_root("terminal-no-cookie-happy-path-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "no-cookie-happy-path");
        let app = router(st);

        async fn assert_reached_real_logic(app: Router, req: Request<Body>, label: &str) {
            let resp = app.oneshot(req).await.unwrap();
            let body = body_string(resp).await;
            assert!(
                !body.is_empty(),
                "{label}: an empty body with no cookie means the old auth refusal is still gating this route"
            );
        }

        assert_reached_real_logic(app.clone(), terminal_req(&project.id, None), "terminal_page").await;
        assert_reached_real_logic(
            app.clone(),
            transcript_page_req(&project.id, None),
            "transcript_page",
        )
        .await;
        assert_reached_real_logic(
            app.clone(),
            terminal_screen_req(&project.id, "no-such-pane", None),
            "terminal_screen",
        )
        .await;
        assert_reached_real_logic(
            app.clone(),
            terminal_transcript_req(&project.id, "no-such-pane", None, None),
            "terminal_transcript",
        )
        .await;
        assert_reached_real_logic(
            app.clone(),
            terminal_input_req(&project.id, "no-such-pane", "hi", Some(true), None),
            "terminal_input",
        )
        .await;
        assert_reached_real_logic(
            app.clone(),
            terminal_keys_req(&project.id, "no-such-pane", &["enter"], None),
            "terminal_keys",
        )
        .await;
        assert_reached_real_logic(app.clone(), create_pane_req(&project.id, None), "terminal_create_pane")
            .await;
        assert_reached_real_logic(
            app.clone(),
            create_agent_req(&project.id, &serde_json::json!({ "preset": "unconfigured" }), None),
            "terminal_create_agent",
        )
        .await;
        assert_reached_real_logic(app.clone(), unassigned_req(None), "unassigned_terminal_page").await;
        assert_reached_real_logic(
            app.clone(),
            unassigned_screen_req("no-such-pane", None),
            "unassigned_terminal_screen",
        )
        .await;
        assert_reached_real_logic(
            app.clone(),
            unassigned_input_req("no-such-pane", "hi", true, None),
            "unassigned_terminal_input",
        )
        .await;
        assert_reached_real_logic(
            app,
            unassigned_keys_req("no-such-pane", &["enter"], None),
            "unassigned_terminal_keys",
        )
        .await;

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

        let switch_resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::json!({ "enabled": true }).to_string()))
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

        let resp = app
            .oneshot(terminal_screen_req(&project_a.id, &outside_agent.pane_id, None))
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

        let resp = app
            .oneshot(terminal_screen_req(&project.id, "no-such-pane", None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_string(resp).await;
        assert!(body.contains("pane not found"), "{body}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    // ---- screen-revision fix: `revision` tracks the screen text itself ----
    //
    // Before this fix, `terminal_screen` returned herdr's own `read.revision`
    // verbatim, which only bumps when the operator's own input is echoed
    // back (see `herdr/fake.rs`'s `read_then_reply_echoes_and_bumps_revision`)
    // — never when the agent under it produces new output on its own. Two
    // live polls three seconds apart on one pane returned different text,
    // both carrying `revision: 0`, so `app.js`'s dedupe
    // (`if (lastRevision[paneId] === body.revision) return;`) discarded
    // every repaint after the first and the pane froze.

    /// The defect this cell fixes: a pane whose screen text changed between
    /// two polls must report a DIFFERENT `revision` each time — this failed
    /// against the pre-fix code, which echoed herdr's own unchanging
    /// `read.revision` straight through.
    #[tokio::test]
    async fn terminal_screen_reports_a_changed_revision_when_the_screen_changes() {
        let dir = fresh_root("terminal-screen-revision-changed");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-revision-changed-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.seed_scroll_pane(&started.pane_id, "first frame", "first frame", None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project = register(&st, &root, "screen-revision-changed");
        let app = router(st);

        let first = app
            .clone()
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        let first_json: serde_json::Value =
            serde_json::from_str(&body_string(first).await).unwrap();

        // The agent produced new output on its own — no input was sent, so
        // herdr's own `read.revision` would stay put.
        fake.seed_scroll_pane(&started.pane_id, "second frame", "second frame", None);

        let second = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        let second_json: serde_json::Value =
            serde_json::from_str(&body_string(second).await).unwrap();

        assert_ne!(
            first_json["revision"], second_json["revision"],
            "changed output must report a different revision: {first_json} vs {second_json}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// A document path an agent printed comes back as a link into this same
    /// project, opening in its own tab — the screen naming a spec is one
    /// click from the spec. Only markdown under `docs/` qualifies: mdview
    /// renders markdown, so a directory or an image would link to a page that
    /// does not exist.
    #[tokio::test]
    async fn terminal_screen_links_the_doc_paths_an_agent_printed() {
        let dir = fresh_root("terminal-screen-doc-links");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-doc-links-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let screen = "wrote docs/specs/agent-terminal.md and docs/assets/logo.png\n";
        fake.seed_scroll_pane(&started.pane_id, screen, screen, None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project = register(&st, &root, "screen-doc-links");
        let app = router(st);

        let resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
        let text = json["text"].as_str().unwrap();

        assert!(
            text.contains(&format!(
                r#"href="/p/{}/docs/specs/agent-terminal.md""#,
                project.id
            )),
            "the markdown path must link into this project: {text}"
        );
        assert!(text.contains(r#"target="_blank""#), "{text}");
        assert!(
            !text.contains("logo.png\""),
            "a non-markdown path must not become a link: {text}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The screen route serves herdr's scrollback, not just the visible
    /// frame: a pane with more history than fits on screen answers with the
    /// older lines too, capped at [`SCREEN_READ_LINES`]. This failed against
    /// the pre-change code, which asked for `Visible` and could only ever
    /// return the one frame — the whole reason the box looked "limited".
    #[tokio::test]
    async fn terminal_screen_serves_scrollback_capped_at_the_read_limit() {
        let dir = fresh_root("terminal-screen-scrollback");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-scrollback-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        // A pane whose scrollback runs well past both the visible frame and
        // the read limit, every line individually identifiable.
        let visible = "line 499";
        let recent: String = (0..500)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        fake.seed_scroll_pane(&started.pane_id, visible, &recent, None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project = register(&st, &root, "screen-scrollback");
        let app = router(st);

        let resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
        let text = json["text"].as_str().unwrap();

        assert_eq!(
            text.lines().count(),
            SCREEN_READ_LINES,
            "the screen must carry exactly the requested tail: {text}"
        );
        // The oldest line still served, and the one just before it dropped —
        // a tail of exactly SCREEN_READ_LINES out of 500.
        assert!(
            text.contains("line 300"),
            "scrollback beyond the visible frame must be served: {text}"
        );
        assert!(
            !text.contains("line 299"),
            "the read must stop at the cap, not serve the whole buffer: {text}"
        );
        assert!(
            text.contains("line 499"),
            "the newest line must still be there: {text}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The guard this cell must not break: a pane whose screen text did NOT
    /// change between two polls reports the SAME `revision` both times, so
    /// `app.js`'s dedupe still skips the redundant repaint.
    #[tokio::test]
    async fn terminal_screen_reports_the_same_revision_when_the_screen_is_unchanged() {
        let dir = fresh_root("terminal-screen-revision-unchanged");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-revision-unchanged-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.seed_scroll_pane(&started.pane_id, "steady frame", "steady frame", None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "screen-revision-unchanged");
        let app = router(st);

        let first = app
            .clone()
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        let first_json: serde_json::Value =
            serde_json::from_str(&body_string(first).await).unwrap();

        let second = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        let second_json: serde_json::Value =
            serde_json::from_str(&body_string(second).await).unwrap();

        assert_eq!(
            first_json["revision"], second_json["revision"],
            "unchanged output must keep reporting the same revision: {first_json} vs {second_json}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Edge case: an empty screen reports a stable revision rather than
    /// panicking or reporting a bare `0` used as a sentinel.
    #[tokio::test]
    async fn terminal_screen_of_an_empty_pane_reports_a_stable_non_sentinel_revision() {
        let dir = fresh_root("terminal-screen-revision-empty");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-revision-empty-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.seed_scroll_pane(&started.pane_id, "", "", None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "screen-revision-empty");
        let app = router(st);

        let resp = app
            .clone()
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_str(&body_string(resp).await).unwrap();
        assert_eq!(json["text"], serde_json::json!(""), "{json}");
        assert_ne!(
            json["revision"],
            serde_json::json!(0),
            "an empty screen's revision must not collapse to the 0 sentinel: {json}"
        );

        let resp2 = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_str(&body_string(resp2).await).unwrap();
        assert_eq!(
            json["revision"], json2["revision"],
            "an empty screen's revision must stay stable across polls: {json} vs {json2}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Edge case: two different panes carrying identical text do not
    /// collide in a way that suppresses either one's updates — each pane's
    /// `revision` is compared against its own last-seen value client-side
    /// (`lastRevision[paneId]`), so two panes sharing a revision value is
    /// fine; what would NOT be fine is either pane failing to report a
    /// changed revision once ITS OWN text changes.
    #[tokio::test]
    async fn terminal_screen_two_panes_with_identical_text_each_still_update_independently() {
        let dir = fresh_root("terminal-screen-revision-two-panes");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-revision-two-panes-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let pane_a = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let pane_b = fake
            .agent_start("w2", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.seed_scroll_pane(&pane_a.pane_id, "shared frame", "shared frame", None);
        fake.seed_scroll_pane(&pane_b.pane_id, "shared frame", "shared frame", None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project = register(&st, &root, "screen-revision-two-panes");
        let app = router(st);

        let a_first = app
            .clone()
            .oneshot(terminal_screen_req(&project.id, &pane_a.pane_id, None))
            .await
            .unwrap();
        let a_first_json: serde_json::Value =
            serde_json::from_str(&body_string(a_first).await).unwrap();
        let b_first = app
            .clone()
            .oneshot(terminal_screen_req(&project.id, &pane_b.pane_id, None))
            .await
            .unwrap();
        let b_first_json: serde_json::Value =
            serde_json::from_str(&body_string(b_first).await).unwrap();

        // Identical text on both panes is expected to agree — that's not a
        // collision, since the client keys its dedupe by pane id.
        assert_eq!(a_first_json["revision"], b_first_json["revision"]);

        // Pane A's own output changes; pane B's does not.
        fake.seed_scroll_pane(&pane_a.pane_id, "pane a moved on", "pane a moved on", None);

        let a_second = app
            .clone()
            .oneshot(terminal_screen_req(&project.id, &pane_a.pane_id, None))
            .await
            .unwrap();
        let a_second_json: serde_json::Value =
            serde_json::from_str(&body_string(a_second).await).unwrap();
        let b_second = app
            .oneshot(terminal_screen_req(&project.id, &pane_b.pane_id, None))
            .await
            .unwrap();
        let b_second_json: serde_json::Value =
            serde_json::from_str(&body_string(b_second).await).unwrap();

        assert_ne!(
            a_first_json["revision"], a_second_json["revision"],
            "pane A's own changed output must report a different revision"
        );
        assert_eq!(
            b_first_json["revision"], b_second_json["revision"],
            "pane B's unchanged output must keep reporting the same revision, unaffected by pane A"
        );

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

        let resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["text"], serde_json::json!(screen_text), "{body}");
        // Since the screen-revision fix, `revision` is a stateless hash of
        // the raw screen text (`mdview_core::ansi::revision_of`), not
        // herdr's own field — see `terminal_screen_reports_a_changed_revision_when_the_screen_changes`
        // below for the defect this replaced.
        assert_eq!(
            json["revision"],
            serde_json::json!(mdview_core::ansi::revision_of(screen_text)),
            "{body}"
        );

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

        let resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
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

    /// terminal-scroll-2: with no `?history` query, `terminal_screen` must
    /// take exactly today's path — the plain `ReadSource::Recent` read,
    /// never `PaneScroller::read_history` — proven here by the fake's
    /// `sent_text_log` staying empty (`PaneScroller` is the only thing that
    /// ever calls `send_text`, see `pane_scroller.rs`'s own doc comment).
    #[tokio::test]
    async fn terminal_screen_without_history_param_never_touches_pane_scroller() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("terminal-screen-no-history");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-no-history-project");
        let fake = std::sync::Arc::new(FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let text = "live frame\n❯ ";
        fake.seed_scroll_pane(&started.pane_id, text, text, Some("older frame\n❯ "));

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project = register(&st, &root, "screen-no-history");
        let app = router(st);

        let resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["text"], serde_json::json!(mdview_core::ansi::to_html(text)), "{body}");
        assert!(
            fake.sent_text_log(&started.pane_id).await.is_empty(),
            "an absent history param must never route through PaneScroller"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// terminal-scroll-2: `?history=2` must route through
    /// `PaneScroller::read_history` with 2 pages — proven both by the
    /// content returned (the 2nd escape page, not the 1st or live) and by
    /// the fake's `sent_text_log` recording exactly two `PAGE_UP` hops then
    /// one `RESTORE_BOTTOM`, mirroring `pane_scroller.rs`'s own
    /// `pages_gt_1_hops_back_further_than_a_single_pageup` unit test at the
    /// HTTP layer.
    #[tokio::test]
    async fn terminal_screen_history_param_sends_two_pageups_then_restores() {
        use crate::herdr::fake::FakeHerdr;
        use crate::herdr::pane_scroller::{PAGE_UP, RESTORE_BOTTOM};

        let dir = fresh_root("terminal-screen-history-2");
        enable_terminal(&dir);
        let root = fresh_root("terminal-screen-history-2-project");
        let fake = std::sync::Arc::new(FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let live_bottom = "page0 (live)\n❯ ";
        fake.seed_scroll_pane(
            &started.pane_id,
            live_bottom,
            live_bottom,
            Some("page1 further back\npage0 (live)\n❯ "),
        );
        fake.push_escape_page(
            &started.pane_id,
            "page2 even further back\npage1 further back\npage0 (live)\n❯ ",
        );

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let project = register(&st, &root, "screen-history-2");
        let app = router(st);

        let resp = app
            .oneshot(Request::builder()
                .uri(format!("/p/{}/_terminal/{}/screen?history=2", project.id, started.pane_id))
                .method("GET")
                .body(Body::empty())
                .unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = json["text"].as_str().unwrap();
        assert!(
            text.contains("page2 even further back"),
            "history=2 should land 2 hops back, not just 1: {text}"
        );
        assert_eq!(
            fake.sent_text_log(&started.pane_id).await,
            vec![PAGE_UP.to_string(), PAGE_UP.to_string(), RESTORE_BOTTOM.to_string()],
            "two PageUp hops then one restore-to-bottom, in that order"
        );

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

        let resp = app
            .oneshot(terminal_screen_req(&project.id, "w1:p1", None))
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

    /// terminal-pane-scope D4: a GET to one pane's own transcript page —
    /// `/p/{id}/_transcript/pane/{pane_id}` — the pane-scoped sibling of
    /// `transcript_page_req` above.
    fn transcript_pane_req(id: &str, pane_id: &str, cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder()
            .uri(format!("/p/{id}/_transcript/pane/{pane_id}"))
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

    /// D12: with the D7 `terminal.enabled` switch off, the Transcript tab's
    /// page answers with the ordinary not-found page and its data endpoint
    /// answers with a reasoned JSON 404 — no cookie, no token, nothing but
    /// the switch decides this.
    #[tokio::test]
    async fn transcript_family_disabled_answers_with_the_disabled_shapes() {
        let dir = fresh_root("transcript-family-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("transcript-family-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "transcript-family-disabled");
        let app = router(st);

        let page = app
            .clone()
            .oneshot(transcript_page_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(page.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            page.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8",
            "the disabled Transcript tab must answer the ordinary not-found page, not an empty body"
        );
        let page_body = body_string(page).await;
        assert!(!page_body.is_empty(), "a disabled terminal must never answer with nothing");

        let data = app
            .oneshot(terminal_transcript_req(&project.id, "w1:p1", None, None))
            .await
            .unwrap();
        assert_eq!(data.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            data.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled transcript endpoint must answer JSON, not an empty body"
        );
        let data_body = body_string(data).await;
        assert!(
            data_body.contains("disabled"),
            "the disabled transcript endpoint must name the reason: {data_body}"
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

        let resp = app
            .oneshot(terminal_transcript_req(
                &project_a.id,
                &outside_agent.pane_id,
                None,
                None,
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

        let resp = app
            .oneshot(terminal_transcript_req(&project.id, &started.pane_id, None, None))
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

        // Opening read: backfills the tail.
        let open_resp = app
            .clone()
            .oneshot(terminal_transcript_req(&project.id, &started.pane_id, None, None))
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
                None,
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
                None,
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

        let resp = app
            .oneshot(terminal_transcript_req(
                &project.id,
                &started.pane_id,
                Some("../../etc/passwd.jsonl:0"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&transcript_root).ok();
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

        let resp = app
            .oneshot(transcript_page_req(&project.id, None))
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

    /// toa-5: a field the settings page renders inside `terminal-config-form`
    /// but the JS submit handler's `fetch` body omits is a control the route
    /// accepts but the browser can never actually flip — exactly the defect
    /// `unassigned_enabled` had until this cell (the checkbox rendered,
    /// toggled, and did nothing on save, because the JSON body sent never
    /// carried the key). This proof reads every `name="..."` the form
    /// actually renders straight out of the rendered HTML — not a hand-kept
    /// list that could drift from the form — and requires the handler to
    /// read each one (`form.<name>.checked` or `form.<name>.value`), so a
    /// field added to one side and forgotten on the other fails here instead
    /// of only silently in a browser.
    #[test]
    fn terminal_config_form_submission_carries_every_switch_the_page_renders() {
        let cfg = Config::default();
        let html = views::settings_page(&cfg, false, false, views::NotifyCredentialView::NotConfigured);
        let form_start = html
            .find(r#"id="terminal-config-form""#)
            .expect("the terminal settings form must exist on the settings page");
        let form_end = html[form_start..]
            .find("</form>")
            .map(|i| form_start + i)
            .expect("the terminal settings form must close");
        let form_html = &html[form_start..form_end];

        let mut field_names = Vec::new();
        let mut rest = form_html;
        while let Some(idx) = rest.find(r#"name=""#) {
            let after = &rest[idx + 6..];
            let end = after.find('"').expect("a name attribute must close");
            field_names.push(after[..end].to_string());
            rest = &after[end..];
        }
        assert!(
            !field_names.is_empty(),
            "the terminal settings form must render at least one named field: {form_html}"
        );

        let js = views::APP_JS;
        let handler_start = js
            .find(r#"var form = document.getElementById("terminal-config-form");"#)
            .expect("the terminal config submit handler must exist in assets/app.js");
        let handler_end = js[handler_start..]
            .find("})();")
            .map(|i| handler_start + i)
            .expect("the submit handler's IIFE must close");
        let handler = &js[handler_start..handler_end];

        for name in &field_names {
            assert!(
                handler.contains(&format!("form.{name}.")),
                "the settings page renders a `{name}` field but the JSON submit handler \
                 never reads it (expected `form.{name}.checked` or `form.{name}.value`), so \
                 that control cannot actually flip the switch even though the route accepts \
                 it: {handler}"
            );
        }
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

        let resp = app
            .clone()
            .oneshot(terminal_input_req(
                &project.id,
                &started.pane_id,
                "draft reply",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ok_body = body_string(resp).await;
        assert!(ok_body.contains("\"ok\":true"), "{ok_body}");

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
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

        let resp = app
            .clone()
            .oneshot(terminal_input_req(
                &project.id,
                &started.pane_id,
                "go ahead",
                Some(true),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
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

        let resp = app
            .clone()
            .oneshot(terminal_keys_req(
                &project.id,
                &started.pane_id,
                &["down", "enter"],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &started.pane_id, None))
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

        let input_resp = app
            .clone()
            .oneshot(terminal_input_req(
                &project_a.id,
                &outside_agent.pane_id,
                "should never land",
                Some(true),
                None,
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
                None,
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

    /// D12: with the D7 `terminal.enabled` switch off, both write endpoints
    /// answer with the family's reasoned JSON 404 — no cookie, no token,
    /// nothing but the switch decides this.
    #[tokio::test]
    async fn terminal_write_routes_disabled_answer_with_a_reasoned_json_404() {
        let dir = fresh_root("terminal-write-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("terminal-write-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "write-disabled");
        let app = router(st);

        let input = app
            .clone()
            .oneshot(terminal_input_req(&project.id, "w1:p1", "hi", Some(true), None))
            .await
            .unwrap();
        assert_eq!(input.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            input.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled input endpoint must answer JSON, not an empty body"
        );
        let input_body = body_string(input).await;
        assert!(
            input_body.contains("disabled"),
            "the disabled input endpoint must name the reason: {input_body}"
        );

        let keys = app
            .oneshot(terminal_keys_req(&project.id, "w1:p1", &["enter"], None))
            .await
            .unwrap();
        assert_eq!(keys.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            keys.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled keys endpoint must answer JSON, not an empty body"
        );
        let keys_body = body_string(keys).await;
        assert!(
            keys_body.contains("disabled"),
            "the disabled keys endpoint must name the reason: {keys_body}"
        );

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

        let resp = app
            .oneshot(terminal_req(&project.id, None))
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
        // The reply box holds more than one line: a message worth sending to
        // an agent rarely fits on one, and a single-line field also stole
        // Enter from the text itself.
        assert!(
            body.contains("<textarea class=\"term-reply__text\""),
            "the reply box must be a multi-line field: {body}"
        );
        assert!(
            !body.contains("<input type=\"text\" class=\"term-reply__text\""),
            "the single-line reply field must be gone: {body}"
        );
        // The card reads in the order an operator reaches it: the screen with
        // its own two scroll controls riding on it, then the keys that drive
        // the agent, then the box they write in — with its send row under it,
        // not squeezed beside it.
        let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("missing {needle}: {body}"));
        let screen = at("class=\"term-screen\"");
        let scroll = at("class=\"term-scroll\"");
        let arrows = at("class=\"term-keys term-keys--move\"");
        let reply = at("class=\"term-reply\"");
        let actions = at("class=\"term-reply__actions\"");
        assert!(
            screen < scroll && scroll < arrows && arrows < reply && reply < actions,
            "pane controls out of order (screen {screen}, scroll {scroll}, arrows {arrows}, reply {reply}, actions {actions}): {body}"
        );
        // The scroll pair belongs to the screen, inside its wrapper — not to
        // the key row, which is where it used to live.
        assert!(
            at("class=\"term-screen-wrap\"") < screen && scroll < at("class=\"term-controls\""),
            "the scroll pair must ride on the screen, not sit in the key row: {body}"
        );
        // The arrows and the named keys now share one line inside a single
        // control block — the second row they used to sit in is gone, and its
        // wrapper must not come back, or the line splits in two again.
        assert!(
            body.contains("class=\"term-controls\"") && !body.contains("term-controls__row"),
            "the controls must sit in one single-row block: {body}"
        );
        // Both key groups are inside that block, the arrows first.
        let named = body[arrows + 1..]
            .find("class=\"term-keys\"")
            .unwrap_or_else(|| panic!("missing the named-key group: {body}"))
            + arrows
            + 1;
        assert!(
            named < reply,
            "the named keys must sit in the control block, above the reply box: {body}"
        );

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

    // ---- project-suggestions: suggested folders on the Projects page ----

    /// Truth: with `terminal.enabled` off, `GET /` carries no suggestion
    /// block and no filesystem path — this cell's gate deliberately reads
    /// only `terminal_family_enabled`, so a stray session running while the
    /// switch is off must leave no trace at all, not even the section
    /// itself.
    #[tokio::test]
    async fn suggestions_switch_off_carries_no_block_and_no_path() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-switch-off");
        let scratch = fresh_root("suggest-switch-off-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let resp = get(router(st), "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            !body.contains("proj-suggestions") && !body.contains("Suggested projects"),
            "the switch being off must render no suggestion block at all: {body}"
        );
        assert!(
            !body.contains(&stray_root.to_string_lossy().to_string()),
            "the switch being off must never leak a filesystem path: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Truth: a pane inside a registered project is never suggested; a pane
    /// outside every project is. The registered pane sits in a
    /// subdirectory of the project root (not the root itself), proving the
    /// D2 containment check, not merely a root-string comparison, is what
    /// excludes it.
    #[tokio::test]
    async fn suggestions_partition_owned_panes_from_a_genuinely_stray_one() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-partition");
        enable_terminal(&dir);
        let scratch = fresh_root("suggest-partition-scratch");
        let project_root = scratch.join("owned-project");
        let owned_sub = project_root.join("sub");
        let stray_root = scratch.join("stray-agent-cwd");
        std::fs::create_dir_all(&owned_sub).unwrap();
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some(&owned_sub.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        register(&st, &project_root, "owned-project");
        let resp = get(router(st), "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&stray_root.to_string_lossy().to_string()),
            "a pane outside every registered project must be suggested: {body}"
        );
        assert!(
            !body.contains(&owned_sub.to_string_lossy().to_string()),
            "a pane already inside a registered project's boundary must never be suggested: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Truth: a pane with no agent at all (a plain shell) in an unregistered
    /// folder is still suggested — `unassigned_panes` iterates
    /// `snapshot.agents` and would miss this case entirely, which is exactly
    /// why `suggested_projects` reads `snapshot.panes` instead.
    #[tokio::test]
    async fn suggestions_include_a_shell_pane_with_no_agent() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-shell-only");
        enable_terminal(&dir);
        let stray_root = fresh_root("suggest-shell-only-stray");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.tab_create("w1", Some(&stray_root.to_string_lossy()))
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let resp = get(router(st), "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&stray_root.to_string_lossy().to_string()),
            "a plain shell pane (no agent) in an unregistered folder must still be suggested: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&stray_root).ok();
    }

    /// Truth: two sessions in one folder produce one suggestion carrying a
    /// count of two, not two rows.
    #[tokio::test]
    async fn suggestions_dedup_two_sessions_in_one_folder_to_one_row_with_count_two() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-dedup");
        enable_terminal(&dir);
        let stray_root = fresh_root("suggest-dedup-stray");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.agent_start("w1", Some(&stray_root.to_string_lossy()), &["codex".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let resp = get(router(st), "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let path = stray_root.to_string_lossy().to_string();
        // Each suggestion row carries the path twice — once as display text,
        // once as the register form's hidden `value` — so a single,
        // deduped row shows the path exactly twice; two undeduped rows
        // would show it four times.
        assert_eq!(
            body.matches(&path).count(),
            2,
            "two sessions in the same folder must dedup to one row: {body}"
        );
        assert!(
            body.contains("2 sessions"),
            "the deduped row must carry a session count of two: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&stray_root).ok();
    }

    /// Truth: a pane whose cwd herdr reports as empty is dropped rather than
    /// suggested as a blank path.
    #[tokio::test]
    async fn suggestions_drop_a_pane_whose_cwd_is_empty() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-empty-cwd");
        enable_terminal(&dir);
        let stray_root = fresh_root("suggest-empty-cwd-stray");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.set_pane_dirs(&started.pane_id, None, None).await.unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let resp = get(router(st), "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            !body.contains(&stray_root.to_string_lossy().to_string()),
            "a pane with no cwd at all must never render as a blank-path suggestion: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&stray_root).ok();
    }

    /// Truth: a directory that is exactly a registered project's root never
    /// appears as a suggestion, even though a session is running there.
    #[tokio::test]
    async fn suggestions_never_include_a_registered_projects_own_root() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-own-root");
        enable_terminal(&dir);
        let project_root = fresh_root("suggest-own-root-project");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some(&project_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        register(&st, &project_root, "own-root");
        let resp = get(router(st), "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            !body.contains(&project_root.to_string_lossy().to_string()),
            "a registered project's own root must never appear as a suggestion: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&project_root).ok();
    }

    /// Truth: when one registered project's root cannot construct a
    /// `Boundary`, the suggestion block is empty, not populated — the same
    /// fail-closed rule `unassigned_panes` follows, proven the same way
    /// `unassigned_group_fails_closed_when_a_projects_boundary_is_unconstructable`
    /// proves it for the Unassigned group.
    #[tokio::test]
    async fn suggestions_fail_closed_when_a_projects_boundary_is_unconstructable() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-fail-closed");
        enable_terminal(&dir);
        let scratch = fresh_root("suggest-fail-closed-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();
        let denied_root = PathBuf::from("/etc/mdview-test-fixture-nonexistent");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &denied_root, "denied-root-project");
        assert_eq!(
            project.root_path, denied_root,
            "sanity: canonicalize falls back to the literal path when it doesn't exist"
        );
        let resp = get(router(st), "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            !body.contains(&stray_root.to_string_lossy().to_string()),
            "a project whose boundary cannot be constructed must fail the whole suggestion list \
             closed to zero, not leak a stray pane's path: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// End to end (truth): posting a suggestion's path registers it, and it
    /// appears as a project row — no longer a suggestion — on the next load.
    #[tokio::test]
    async fn suggestions_posting_a_suggested_path_registers_it_as_a_project() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-end-to-end");
        enable_terminal(&dir);
        let stray_root = fresh_root("suggest-end-to-end-stray");
        write(&stray_root, "README.md", "# Stray\n");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let engine = st.engine.clone();
        let app = router(st);

        let before = get(app.clone(), "/").await;
        let before_body = body_string(before).await;
        assert!(
            before_body.contains(&stray_root.to_string_lossy().to_string()),
            "sanity: the stray folder must render as a suggestion first: {before_body}"
        );

        let register_req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "path={}",
                urlencoding_lite(&stray_root.to_string_lossy())
            )))
            .unwrap();
        let register_resp = app.clone().oneshot(register_req).await.unwrap();
        assert_eq!(register_resp.status(), StatusCode::SEE_OTHER);
        assert_eq!(register_resp.headers().get(header::LOCATION).unwrap(), "/");
        assert_eq!(
            engine.list_projects().unwrap().len(),
            1,
            "the suggested path must now be registered"
        );

        let after = get(app, "/").await;
        let after_body = body_string(after).await;
        assert!(
            after_body.contains("href=\"/p/"),
            "the newly registered project must appear as a project row: {after_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&stray_root).ok();
    }

    /// Refusal parity (truth): a suggestion whose path is deny-listed is
    /// refused by the register route's own fixed-code guard, exactly like a
    /// hand-typed path — `suggested_projects` runs no deny-list check of its
    /// own, so `/etc` (guaranteed to exist, same fixture the register-route
    /// deny-list tests use) still renders as a suggestion, and posting it
    /// gets the same `denied` refusal `register_project_refuses_a_root_on_the_hard_deny_list`
    /// proves for a hand-typed submission.
    #[tokio::test]
    async fn suggestions_deny_listed_path_is_refused_by_the_register_route_itself() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("suggest-deny-listed");
        enable_terminal(&dir);

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some("/etc"), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let engine = st.engine.clone();
        let app = router(st);

        let resp = get(app.clone(), "/").await;
        let body = body_string(resp).await;
        assert!(
            body.contains("/etc"),
            "suggestion computation must not pre-filter a deny-listed path: {body}"
        );

        let register_req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("path=%2Fetc"))
            .unwrap();
        let register_resp = app.oneshot(register_req).await.unwrap();
        assert_eq!(
            register_resp.headers().get(header::LOCATION).unwrap(),
            "/?register_error=denied",
            "a deny-listed suggestion must be refused by the register route's own code"
        );
        assert_eq!(
            engine.list_projects().unwrap().len(),
            0,
            "a deny-listed suggestion must never register"
        );

        std::fs::remove_dir_all(&dir).ok();
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

    /// D12: with the D7 `terminal.enabled` switch off, `/_terminal/unassigned`
    /// answers with mdview's ordinary not-found page — no cookie, no token,
    /// nothing but the switch decides this. toa-1's deviation from the
    /// plan's keep-list (recorded in this cell's outcome): the keep-list
    /// names six disabled-state tests and omits this one, but it is the only
    /// disabled-state coverage this feature has for the Unassigned group's
    /// own *page* route (`unassigned_screen`/`unassigned_write` cover its
    /// data routes) — retiring it would leave that page's D7 gate unproven.
    #[tokio::test]
    async fn unassigned_family_disabled_answers_with_the_ordinary_not_found_page() {
        let dir = fresh_root("unassigned-family-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let disabled = app.oneshot(unassigned_req(None)).await.unwrap();

        assert_eq!(disabled.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            disabled.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8",
            "a disabled terminal must answer the ordinary not-found page, not an empty body"
        );
        let body = body_string(disabled).await;
        assert!(!body.is_empty(), "a disabled terminal must never answer with nothing");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// toa-4 (D9), the truth this cell exists to prove: the Unassigned
    /// group's own switch is ANDed with the D7 family switch (`enabled`),
    /// never a substitute for it and never substituted by it. Every one of
    /// the group's four routes — page and the three data routes — answers
    /// not-found unless BOTH switches are on; turning on `terminal.enabled`
    /// alone (the switch every install already knows) must not be read as
    /// turning on a group that reaches every herdr pane on the host.
    #[tokio::test]
    async fn unassigned_group_answers_not_found_unless_both_its_switches_are_on() {
        async fn assert_all_not_found(app: &Router, label: &str) {
            let page = app.clone().oneshot(unassigned_req(None)).await.unwrap();
            assert_eq!(page.status(), StatusCode::NOT_FOUND, "{label}: page route");
            let screen = app
                .clone()
                .oneshot(unassigned_screen_req("no-such-pane", None))
                .await
                .unwrap();
            assert_eq!(screen.status(), StatusCode::NOT_FOUND, "{label}: screen route");
            let input = app
                .clone()
                .oneshot(unassigned_input_req("no-such-pane", "hi", true, None))
                .await
                .unwrap();
            assert_eq!(input.status(), StatusCode::NOT_FOUND, "{label}: input route");
            let keys = app
                .clone()
                .oneshot(unassigned_keys_req("no-such-pane", &["enter"], None))
                .await
                .unwrap();
            assert_eq!(keys.status(), StatusCode::NOT_FOUND, "{label}: keys route");
        }

        // Neither switch: the default a fresh install and an install that
        // has never heard of this switch both start from.
        let dir_neither = fresh_root("unassigned-both-switches-neither");
        let app_neither = router(build_state_with_dir(&dir_neither));
        assert_all_not_found(&app_neither, "neither switch").await;
        std::fs::remove_dir_all(&dir_neither).ok();

        // The family switch on, the group's own switch still off — the
        // case D9 exists for. An owner who only ever turned on
        // `terminal.enabled` must not have silently opened this group.
        let dir_family_only = fresh_root("unassigned-both-switches-family-only");
        enable_terminal(&dir_family_only);
        let app_family_only = router(build_state_with_dir(&dir_family_only));
        assert_all_not_found(&app_family_only, "family switch only").await;
        std::fs::remove_dir_all(&dir_family_only).ok();

        // The group's own switch on, the family switch off — the group is
        // still part of the terminal family; its own switch does not
        // resurrect it while the family itself is off.
        let dir_group_only = fresh_root("unassigned-both-switches-group-only");
        enable_unassigned_group(&dir_group_only);
        let app_group_only = router(build_state_with_dir(&dir_group_only));
        assert_all_not_found(&app_group_only, "group switch only").await;
        std::fs::remove_dir_all(&dir_group_only).ok();

        // Both on: the group answers for real — the page route succeeds,
        // and a data route reaches its own real logic (a pane-not-found
        // refusal naming the pane, not the disabled shape naming the
        // switch), proving both switches together restore the group to
        // exactly how it behaves in the rest of this module's coverage.
        let dir_both = fresh_root("unassigned-both-switches-both");
        enable_terminal(&dir_both);
        enable_unassigned_group(&dir_both);
        let app_both = router(build_state_with_dir(&dir_both));
        let page_both = app_both.clone().oneshot(unassigned_req(None)).await.unwrap();
        assert_eq!(
            page_both.status(),
            StatusCode::OK,
            "both switches on: the page route must answer"
        );
        let screen_both = app_both
            .oneshot(unassigned_screen_req("no-such-pane", None))
            .await
            .unwrap();
        assert_eq!(screen_both.status(), StatusCode::NOT_FOUND);
        let screen_both_body = body_string(screen_both).await;
        assert!(
            screen_both_body.contains("pane not found"),
            "both switches on: expected the real pane-not-found refusal, not the disabled reason: {screen_both_body}"
        );
        std::fs::remove_dir_all(&dir_both).ok();
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
        enable_unassigned_group(&dir); // toa-4 (D9): needed for the group's routes to run.
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

        let unassigned_resp = app
            .clone()
            .oneshot(unassigned_req(None))
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
            .oneshot(terminal_req(&project.id, None))
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

    /// The project list is rows, and a worktree is nested under the project
    /// it branches from — whatever order the registry hands them back in. The
    /// registry orders by last-seen, so the branch is registered FIRST here:
    /// nesting that only worked when the parent happened to come first would
    /// pass a weaker test and fail on the real page.
    #[tokio::test]
    async fn home_page_lists_projects_as_rows_with_each_worktree_under_its_parent() {
        let dir = fresh_root("home-worktree-rows");
        let st = build_state_with_dir(&dir);

        // Registered deliberately out of order: branch, orphan branch, parent.
        let branch_root = dir.join("demo--wt--alpha");
        let orphan_root = dir.join("nowhere--wt--solo");
        let parent_root = dir.join("demo");
        for r in [&branch_root, &orphan_root, &parent_root] {
            std::fs::create_dir_all(r).unwrap();
        }
        let branch = register(&st, &branch_root, "demo--wt--alpha");
        let orphan = register(&st, &orphan_root, "nowhere--wt--solo");
        let parent = register(&st, &parent_root, "demo");

        let body = body_string(get(router(st), "/").await).await;

        // Rows, not the old card grid.
        assert!(
            body.contains("<ul class=\"proj-list\">") && !body.contains("class=\"proj-card\""),
            "the project list must render as rows: {body}"
        );

        let at = |id: &str| {
            body.find(&format!("href=\"/p/{id}/\""))
                .unwrap_or_else(|| panic!("{id} must be listed: {body}"))
        };
        // The parent leads its own branch even though the branch was seen first.
        assert!(
            at(&parent.id) < at(&branch.id),
            "a worktree must follow the project it branches from: {body}"
        );
        // Every project appears exactly once — nesting must not duplicate a row.
        assert_eq!(
            body.matches(&format!("href=\"/p/{}/\"", branch.id)).count(),
            1,
            "a nested worktree must be listed once, not twice: {body}"
        );

        // The branch row is marked as one and shows only the branch's own name.
        assert!(
            body.contains("proj-row proj-row--branch"),
            "a worktree row must be marked as a branch: {body}"
        );
        assert!(
            body.contains("<span class=\"proj-row__name\">alpha</span>"),
            "a branch row must show only the branch name, not the parent's: {body}"
        );

        // An orphan branch has no parent to nest under, so it keeps its full
        // name at the top level rather than hiding under a row that is absent.
        assert!(
            body.contains("<span class=\"proj-row__name\">nowhere--wt--solo</span>"),
            "a worktree whose parent is unregistered must keep its full name: {body}"
        );
        let orphan_row_open = body[..at(&orphan.id)]
            .rfind("<li class=\"")
            .expect("every row opens an <li>");
        assert!(
            body[orphan_row_open..at(&orphan.id)].contains("<li class=\"proj-row\">"),
            "an orphan worktree must not be rendered as someone's branch: {body}"
        );

        // Unchanged rule: this page never carries a filesystem path.
        assert!(
            !body.contains(&dir.to_string_lossy().to_string()),
            "the project list must not leak a filesystem path: {body}"
        );
    }

    /// A last-seen time reads to the minute and no further. The full instant
    /// stays in `datetime` for the script and for any machine reader, but the
    /// text a person sees carries no seconds and no sub-second digits — those
    /// were pushing the project's own name off the line.
    #[tokio::test]
    async fn home_page_shows_a_last_seen_time_cut_to_the_minute() {
        let dir = fresh_root("home-short-instant");
        let st = build_state_with_dir(&dir);
        let root = dir.join("demo");
        std::fs::create_dir_all(&root).unwrap();
        let p = register(&st, &root, "demo");

        let body = body_string(get(router(st), "/").await).await;

        let full = &p.last_seen_at;
        let (date, rest) = full.split_once('T').expect("registry writes an RFC3339 instant");
        let minute = format!("{date} {}", &rest[..5]);
        assert!(
            body.contains(&format!("datetime=\"{full}\"")),
            "the full instant must stay in datetime: {body}"
        );
        assert!(
            body.contains(&format!(">{minute}</time>")),
            "the visible text must read to the minute ({minute}): {body}"
        );
        // The tail past the minute — seconds and sub-second digits — must not
        // appear as text. It is still present inside the datetime attribute,
        // so the check is anchored on the closing tag.
        assert!(
            !body.contains(&format!("{full}</time>")),
            "the raw instant must not be the visible text: {body}"
        );
    }

    /// projects-home-1 (D6), tightened by projects-home-2, restored to full
    /// strength by projects-home-3: the badges family switch is the same
    /// `terminal_family_enabled` gate every other terminal route already
    /// checks — off, `index_page` must never make a herdr call at all, and
    /// no pane id, program name, or badge markup of any kind may reach `/`.
    /// `HangingHerdr`'s `snapshot()` never resolves (`std::future::pending`),
    /// so a stray call despite the switch being off would still hit
    /// `index_page`'s own `INDEX_HERDR_SNAPSHOT_TIMEOUT` (`server.rs:280`)
    /// before answering `None` — projects-home-2 swapped onto this double for
    /// exactly that reason, but then deleted the three assertions that would
    /// have proven anything real leaked, leaving only
    /// `!body.contains("proj-row__badges")`, which the timed-out `None` path
    /// also satisfies. Restored here, plus the elapsed-time assertion
    /// projects-home-2 never added: without it, a stray call still passes
    /// this test, just slowly (after the ~2s timeout) — only a fast response
    /// proves the switch was checked before herdr was ever touched.
    /// `started`'s pane comes from a *separate* `FakeHerdr` never wired into
    /// `st.herdr` — it exists only to give this test real values to assert
    /// absent, since `HangingHerdr` itself tracks no panes.
    #[tokio::test]
    async fn home_page_carries_no_pane_badges_when_terminal_switch_is_off() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-badges-switch-off");
        let scratch = fresh_root("home-badges-switch-off-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let fixture = std::sync::Arc::new(FakeHerdr::new());
        let started = fixture
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = Arc::new(HangingHerdr);
        register(&st, &root, "demo");
        let app = router(st);

        let start = std::time::Instant::now();
        let body = body_string(get(app, "/").await).await;
        let elapsed = start.elapsed();

        assert!(
            !body.contains("proj-row__badges")
                && !body.contains(&started.pane_id)
                && !body.contains("claude"),
            "the switch off must carry no pane id, program name, or badge markup: {body}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "the switch off must never reach herdr at all — a stray call \
             behind it would still return `None` after \
             INDEX_HERDR_SNAPSHOT_TIMEOUT and let the assertion above pass \
             anyway, so only a fast response proves the switch was honoured: \
             took {elapsed:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D1/D2/D3/D5: a parent project, its own worktree branch, and an
    /// unrelated sibling each hold exactly one pane. Every row must badge
    /// only the pane whose working directory sits inside *that row's own*
    /// D2 boundary — a worktree row badges its own panes, never its
    /// parent's, and no project's pane leaks onto a sibling's row.
    #[tokio::test]
    async fn home_page_badges_only_panes_within_each_projects_own_boundary() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-badges-boundary");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-boundary-scratch");
        let parent_root = scratch.join("demo");
        let branch_root = scratch.join("demo--wt--alpha");
        let sibling_root = scratch.join("sibling");
        for r in [&parent_root, &branch_root, &sibling_root] {
            std::fs::create_dir_all(r).unwrap();
        }

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let parent_pane = fake
            .agent_start("w1", Some(&parent_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let branch_pane = fake
            .agent_start("w1", Some(&branch_root.to_string_lossy()), &["codex".to_string()])
            .await
            .unwrap();
        let sibling_pane = fake
            .agent_start("w1", Some(&sibling_root.to_string_lossy()), &["aider".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let parent = register(&st, &parent_root, "demo");
        let branch = register(&st, &branch_root, "demo--wt--alpha");
        let sibling = register(&st, &sibling_root, "sibling");
        let app = router(st);

        let body = body_string(get(app, "/").await).await;
        let href = |project_id: &str, pane_id: &str| format!("/p/{project_id}/_terminal/pane/{pane_id}");

        assert!(
            body.contains(&href(&parent.id, &parent_pane.pane_id)),
            "the parent's own pane is missing from its row: {body}"
        );
        assert!(
            !body.contains(&href(&parent.id, &branch_pane.pane_id)),
            "the branch's pane leaked onto the parent's row: {body}"
        );
        assert!(
            body.contains(&href(&branch.id, &branch_pane.pane_id)),
            "the branch's own pane is missing from its own row: {body}"
        );
        assert!(
            !body.contains(&href(&branch.id, &parent_pane.pane_id)),
            "the parent's pane leaked onto the branch's row: {body}"
        );
        assert!(
            body.contains(&href(&sibling.id, &sibling_pane.pane_id)),
            "the sibling's own pane is missing from its row: {body}"
        );
        assert!(
            !body.contains(&href(&sibling.id, &parent_pane.pane_id))
                && !body.contains(&href(&sibling.id, &branch_pane.pane_id)),
            "another project's pane leaked onto the sibling's row: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D6: a registered project with no pane inside its own boundary must
    /// render exactly as it did before this feature — the row itself, but
    /// no empty badge container standing in for the absence.
    #[tokio::test]
    async fn home_page_project_with_no_pane_in_boundary_renders_without_badge_container() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-badges-empty");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-empty-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = std::sync::Arc::new(FakeHerdr::empty());
        register(&st, &root, "demo");
        let app = router(st);

        let body = body_string(get(app, "/").await).await;
        assert!(
            !body.contains("proj-row__badges"),
            "a project with no pane in its boundary must render no badge container: {body}"
        );
        assert!(
            body.contains("<span class=\"proj-row__name\">demo</span>"),
            "the row itself must still render: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D6: an errored herdr snapshot (`FakeHerdr::set_available(false)`,
    /// `herdr/fake.rs:280`) must still answer `/` with `200` and plain rows
    /// — never a raw error, and never a badge container standing in for the
    /// snapshot it could not take.
    #[tokio::test]
    async fn home_page_renders_plain_rows_when_herdr_is_down() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-badges-herdr-down");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-herdr-down-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.set_available(false);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        register(&st, &root, "demo");
        let app = router(st);

        let resp = get(app, "/").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            !body.contains("proj-row__badges"),
            "a down herdr must render plain rows, not a badge container: {body}"
        );
        assert!(
            body.contains("<span class=\"proj-row__name\">demo</span>"),
            "the row itself must still render when herdr is down: {body}"
        );
        assert!(
            !body.to_lowercase().contains("error"),
            "a down herdr must never surface a raw error on the home page: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// A herdr double whose `snapshot()` never resolves — the one behavior
    /// `FakeHerdr` cannot express (it always answers immediately, `Ok` or
    /// `Err`), but which a real hung daemon can, since `SocketHerdr::call`
    /// carries no timeout of its own on connect, write, or read
    /// (`herdr/socket.rs:198-217`). Every other method is unused by
    /// `index_page` and left unimplemented — reaching one is this test's own
    /// bug, not the code under test's.
    struct HangingHerdr;

    #[async_trait::async_trait]
    impl Herdr for HangingHerdr {
        async fn snapshot(&self) -> herdr::Result<herdr::Snapshot> {
            std::future::pending().await
        }
        async fn ping(&self) -> herdr::Result<herdr::ProtocolInfo> {
            unimplemented!("index_page never pings herdr")
        }
        async fn read_pane(
            &self,
            _pane_id: &str,
            _source: herdr::ReadSource,
            _lines: usize,
        ) -> herdr::Result<herdr::ScreenRead> {
            unimplemented!("index_page never reads a pane")
        }
        async fn send_input(&self, _pane_id: &str, _text: &str, _submit: bool) -> herdr::Result<()> {
            unimplemented!("index_page never sends input")
        }
        async fn send_text(&self, _pane_id: &str, _bytes: &str) -> herdr::Result<()> {
            unimplemented!("index_page never sends text")
        }
        async fn send_keys(&self, _pane_id: &str, _keys: &[String]) -> herdr::Result<()> {
            unimplemented!("index_page never sends keys")
        }
        async fn tab_create(&self, _workspace_id: &str, _cwd: Option<&str>) -> herdr::Result<herdr::TabCreated> {
            unimplemented!("index_page never creates a tab")
        }
        async fn agent_start(
            &self,
            _workspace_id: &str,
            _cwd: Option<&str>,
            _argv: &[String],
        ) -> herdr::Result<herdr::AgentStarted> {
            unimplemented!("index_page never starts an agent")
        }
    }

    /// D6: `index_page` wraps its one herdr snapshot in an explicit
    /// `tokio::time::timeout` precisely because `SocketHerdr::call` has none
    /// of its own — proven here against a herdr double that never answers at
    /// all, not only one that answers with an `Err` (that path is
    /// `home_page_renders_plain_rows_when_herdr_is_down` above).
    #[tokio::test]
    async fn home_page_returns_promptly_when_herdr_hangs() {
        let dir = fresh_root("home-badges-herdr-hang");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-herdr-hang-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = Arc::new(HangingHerdr);
        register(&st, &root, "demo");
        let app = router(st);

        let started = std::time::Instant::now();
        let resp = tokio::time::timeout(Duration::from_secs(10), get(app, "/"))
            .await
            .expect("index_page must return well within its own bounded herdr timeout");
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a hung herdr must not wedge the home page: took {:?}",
            started.elapsed()
        );
        let body = body_string(resp).await;
        assert!(
            !body.contains("proj-row__badges"),
            "a hung herdr must render plain rows, not a badge container: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// A badge's own `href` must resolve on the router (D3) — proven by
    /// actually driving it through `router()`, not just by string shape.
    #[tokio::test]
    async fn home_page_badge_link_resolves_on_the_router() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-badges-link-resolves");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-link-resolves-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "demo");
        let app = router(st);

        let body = body_string(get(app.clone(), "/").await).await;
        let href = format!("/p/{}/_terminal/pane/{}", project.id, started.pane_id);
        assert!(
            body.contains(&href),
            "the badge's own href is missing from the row: {body}"
        );

        let resp = get(app, &href).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "the badge's href must resolve on the router"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// projects-home-1: one project registered on a hard-deny-listed root
    /// (`Boundary::new` refuses to construct on every platform this suite
    /// runs on, same fixture as
    /// `unassigned_group_fails_closed_when_a_projects_boundary_is_unconstructable`
    /// above) sits beside a perfectly healthy one. Only the unconstructable
    /// project's own row loses its badges; the healthy row keeps its own —
    /// the per-project idiom `terminal_page_inner` uses, not
    /// `unassigned_panes`'s whole-group fail-closed shape.
    #[tokio::test]
    async fn home_page_unconstructable_boundary_loses_only_its_own_badges() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-badges-unconstructable");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-unconstructable-scratch");
        let ok_root = scratch.join("demo");
        std::fs::create_dir_all(&ok_root).unwrap();
        let denied_root = PathBuf::from("/etc/mdview-test-fixture-nonexistent-projects-home-1");

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let ok_pane = fake
            .agent_start("w1", Some(&ok_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let denied_pane = fake
            .agent_start("w1", Some(&denied_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let ok_project = register(&st, &ok_root, "demo");
        let denied_project = register(&st, &denied_root, "denied-root-project");
        assert_eq!(
            denied_project.root_path, denied_root,
            "sanity: canonicalize falls back to the literal path when it doesn't exist"
        );
        let app = router(st);

        let body = body_string(get(app, "/").await).await;
        assert!(
            body.contains(&format!(
                "/p/{}/_terminal/pane/{}",
                ok_project.id, ok_pane.pane_id
            )),
            "a healthy project's own badge must still render when a different project's boundary \
             is unconstructable: {body}"
        );
        let denied_row_start = body
            .find(&format!("href=\"/p/{}/\"", denied_project.id))
            .expect("the denied project's own row must still render");
        let denied_row_end = body[denied_row_start..]
            .find("</li>")
            .map(|i| denied_row_start + i + "</li>".len())
            .unwrap_or(body.len());
        assert!(
            !body[denied_row_start..denied_row_end].contains("proj-row__badges"),
            "a project whose own boundary cannot be constructed must lose only its own badges: {body}"
        );
        assert!(
            !body.contains(&denied_pane.pane_id),
            "the denied project's own pane must never badge anywhere on the page: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D2: every pane in the boundary badges whatever its status —
    /// working, idle, done, blocked, and an agent-less shell. D3's own
    /// `status_pill` tints only done/working/blocked, so idle and shell
    /// share the neutral, unmodified dot and are told apart only by the
    /// pill's own text — asserted on that text, never on a modifier class
    /// neither carries. D1a: the badge prints the pane's program (`kind`,
    /// or the literal `shell`), and the agent's own `name` field never
    /// reaches the page.
    #[tokio::test]
    async fn home_page_badges_cover_every_pane_status_and_the_program_never_the_agent_name() {
        use crate::herdr::fake::FakeHerdr;
        use crate::herdr::wire::AgentStatus;

        let dir = fresh_root("home-badges-statuses");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-statuses-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let working = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.set_status(&working.pane_id, AgentStatus::Working).await.unwrap();
        let idle = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["codex".to_string()])
            .await
            .unwrap();
        fake.set_status(&idle.pane_id, AgentStatus::Idle).await.unwrap();
        let done = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["aider".to_string()])
            .await
            .unwrap();
        fake.set_status(&done.pane_id, AgentStatus::Done).await.unwrap();
        let blocked = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["cursor".to_string()])
            .await
            .unwrap();
        fake.set_status(&blocked.pane_id, AgentStatus::Blocked).await.unwrap();
        let shell = fake.tab_create("w1", Some(&root.to_string_lossy())).await.unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "demo");
        let app = router(st);

        let body = body_string(get(app, "/").await).await;
        let href = |pane_id: &str| format!("/p/{}/_terminal/pane/{}", project.id, pane_id);
        for pane_id in [
            &working.pane_id,
            &idle.pane_id,
            &done.pane_id,
            &blocked.pane_id,
            &shell.pane_id,
        ] {
            assert!(
                body.contains(&href(pane_id)),
                "every pane, whatever its status, must badge: {body}"
            );
        }
        for status_text in ["working", "idle", "done", "blocked", "shell"] {
            assert!(
                body.contains(&format!(">{status_text}</span>")),
                "the {status_text} pill's own text must appear: {body}"
            );
        }
        for program in ["claude", "codex", "aider", "cursor", "shell"] {
            assert!(
                body.contains(program),
                "the pane's program {program} must badge: {body}"
            );
        }
        for started in [&working, &idle, &done, &blocked] {
            assert!(
                !body.contains(&started.name),
                "the agent's own name field must never reach the home page: {body}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// projects-home-2: the markup-validity probe the plan declared but the
    /// badge slice never authored. `project_badges` is meant to render as a
    /// sibling of `proj-row__link`, never nested inside it — an anchor
    /// inside an anchor is invalid HTML and browsers unnest it, which would
    /// break the row link itself (D3). Nothing before this test failed if
    /// that sibling relationship were broken; this pins it by locating the
    /// row link's own opening and closing tags and asserting the badge
    /// markup never appears between them.
    #[tokio::test]
    async fn home_page_badges_are_never_nested_inside_the_row_link() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-badges-markup-validity");
        enable_terminal(&dir);
        let scratch = fresh_root("home-badges-markup-validity-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        fake.agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        register(&st, &root, "demo");
        let app = router(st);

        let body = body_string(get(app, "/").await).await;
        assert!(
            body.contains("proj-row__badges"),
            "fixture must actually render a badge for this test to prove anything: {body}"
        );

        let link_start = body
            .find("<a class=\"proj-row__link\"")
            .unwrap_or_else(|| panic!("row link markup missing: {body}"));
        let link_close = body[link_start..]
            .find("</a>")
            .map(|i| link_start + i)
            .unwrap_or_else(|| panic!("row link never closes: {body}"));
        assert!(
            !body[link_start..link_close].contains("proj-row__badges"),
            "the badge block must be a sibling of proj-row__link, never nested inside its anchor: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// The two home-page behaviors that live in `assets/app.js` reach the
    /// markup by class name, so a rename in `views.rs` silently breaks them —
    /// which is exactly what happened when the project cards became rows: the
    /// timestamps stopped being formatted and the delete confirmation stopped
    /// asking. Nothing else in the suite would have caught it, because both
    /// live in a script no route test executes.
    #[tokio::test]
    async fn home_page_script_selectors_match_the_markup_the_page_emits() {
        let dir = fresh_root("home-script-selectors");
        let st = build_state_with_dir(&dir);
        let root = dir.join("demo");
        std::fs::create_dir_all(&root).unwrap();
        register(&st, &root, "demo");

        let body = body_string(get(router(st), "/").await).await;
        let script = include_str!("../assets/app.js");

        for (selector, needle, what) in [
            (
                "time.proj-row__time[datetime]",
                "<time class=\"proj-row__time\" datetime=",
                "the timestamp formatter",
            ),
            (
                ".proj-row__delete",
                "<form class=\"proj-row__delete\"",
                "the delete confirmation",
            ),
        ] {
            assert!(
                script.contains(selector),
                "{what} must still query {selector}"
            );
            assert!(
                body.contains(needle),
                "{what}'s selector {selector} matches nothing the page emits: {body}"
            );
        }
    }

    /// D5/D4's core resolution, on the home page itself: an unauthenticated
    /// `GET /` must never reveal an unassigned agent's name through the
    /// Unassigned card's own markup, even though the group's presence
    /// marker is visible once the D7 switch is on. A second request with
    /// the switch off proves the page renders exactly as it did before this
    /// feature (no marker, no mention of "Unassigned" at all).
    ///
    /// project-suggestions S3 supersedes this test's original cwd claim in
    /// part: with `terminal.enabled` on, the page's separate suggestion
    /// block (gated on that one switch alone, a locked narrowing of D9's
    /// scope recorded in `docs/history/project-suggestions/plan.md`) does
    /// print the stray pane's cwd on purpose — that is the whole feature.
    /// What still holds, and is what this test asserts on the cwd, is
    /// narrower but unchanged: the Unassigned card's own markup carries no
    /// cwd, only presence.
    #[tokio::test]
    async fn unauthenticated_home_page_shows_unassigned_presence_only_and_leaks_nothing() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("home-unassigned-presence");
        // project-suggestions: deliberately not named "...unassigned..." —
        // once `terminal.enabled` is on, the suggestion block (a separate
        // feature, gated on that one switch alone) prints this folder's
        // full path, and this test's own `!contains("unassigned")` checks
        // below must not trip over the literal word appearing inside a
        // scratch *directory name* rather than the Unassigned marker itself.
        let scratch = fresh_root("home-group-presence-scratch");
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

        // toa-4 (D9): the family switch on, the group's own switch still
        // off — the marker must stay absent. Turning on `terminal.enabled`
        // alone must never be read as turning on the Unassigned group.
        enable_terminal(&dir);
        let mut st_family_only = build_state_with_dir(&dir);
        st_family_only.herdr = fake.clone();
        let resp_family_only = get(router(st_family_only), "/").await;
        assert_eq!(resp_family_only.status(), StatusCode::OK);
        let body_family_only = body_string(resp_family_only).await;
        assert!(
            !body_family_only.contains("Unassigned") && !body_family_only.contains("unassigned"),
            "the family switch alone must not surface the Unassigned group's presence marker: {body_family_only}"
        );

        // Both switches on: the presence marker appears, but the pane's own
        // name and cwd never do — this route takes no session and makes no
        // herdr call, so it structurally cannot leak them.
        enable_unassigned_group(&dir);
        let mut st_on = build_state_with_dir(&dir);
        st_on.herdr = fake;
        let resp_on = get(router(st_on), "/").await;
        assert_eq!(resp_on.status(), StatusCode::OK);
        let body_on = body_string(resp_on).await;
        assert!(
            body_on.contains("Unassigned agents"),
            "the group's presence marker is missing once both switches are on: {body_on}"
        );
        assert!(
            !body_on.contains(&stray.name),
            "an unauthenticated home page leaked an unassigned agent's name: {body_on}"
        );
        // project-suggestions S3: the page as a whole now legitimately
        // shows the stray pane's cwd, via the separate suggestion block —
        // that disclosure is this cell's own locked, recorded decision, not
        // a leak. What the Unassigned card itself must still never carry is
        // the pane's cwd, so the assertion narrows to that card's own
        // markup rather than the whole page body.
        let card_start = body_on
            .find(r#"<div class="proj-cards">"#)
            .expect("the Unassigned card must render once both switches are on");
        let card_end = body_on[card_start..]
            .find("</div>")
            .map(|i| card_start + i + "</div>".len())
            .unwrap_or(body_on.len());
        let card_markup = &body_on[card_start..card_end];
        assert!(
            !card_markup.contains(&stray_root.to_string_lossy().to_string()),
            "the Unassigned card's own markup leaked an unassigned agent's cwd: {card_markup}"
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
        enable_unassigned_group(&dir); // toa-4 (D9): needed for the group's routes to run.
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

        // Screen read reaches the stray pane.
        let screen_resp = app
            .clone()
            .oneshot(unassigned_screen_req(&stray.pane_id, None))
            .await
            .unwrap();
        assert_eq!(screen_resp.status(), StatusCode::OK);

        // Screen read refuses the owned pane — it belongs to a project, not
        // to this group.
        let owned_screen_resp = app
            .clone()
            .oneshot(unassigned_screen_req(&owned.pane_id, None))
            .await
            .unwrap();
        assert_eq!(owned_screen_resp.status(), StatusCode::NOT_FOUND);

        // Free-text input reaches the stray pane and is readable back.
        let input_resp = app
            .clone()
            .oneshot(unassigned_input_req(&stray.pane_id, "hello stray", true, None))
            .await
            .unwrap();
        assert_eq!(input_resp.status(), StatusCode::OK);
        let input_body = body_string(input_resp).await;
        assert!(input_body.contains("\"ok\":true"), "{input_body}");

        let after_input = app
            .clone()
            .oneshot(unassigned_screen_req(&stray.pane_id, None))
            .await
            .unwrap();
        let after_input_body = body_string(after_input).await;
        assert!(after_input_body.contains("hello stray"), "{after_input_body}");

        // Named keys reach the stray pane.
        let keys_resp = app
            .clone()
            .oneshot(unassigned_keys_req(&stray.pane_id, &["enter"], None))
            .await
            .unwrap();
        assert_eq!(keys_resp.status(), StatusCode::OK);

        // Input refuses the owned pane too — the write paths honor the same
        // partition the read path and the listing page do.
        let owned_input_resp = app
            .oneshot(unassigned_input_req(&owned.pane_id, "should never land", true, None))
            .await
            .unwrap();
        assert_eq!(owned_input_resp.status(), StatusCode::NOT_FOUND);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Regression (screen-revision fix): the Unassigned group's own screen
    /// endpoint carries the exact same `ScreenBody { text, revision }` shape
    /// as `terminal_screen` and shares the same bug — a pane whose output
    /// changed between two polls must report a DIFFERENT revision, and one
    /// whose output did not change must report the SAME revision.
    #[tokio::test]
    async fn unassigned_terminal_screen_reports_a_changed_revision_when_the_screen_changes() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("unassigned-screen-revision");
        enable_terminal(&dir);
        enable_unassigned_group(&dir);
        let scratch = fresh_root("unassigned-screen-revision-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.seed_scroll_pane(&stray.pane_id, "unassigned first frame", "unassigned first frame", None);

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let app = router(st);

        let first = app
            .clone()
            .oneshot(unassigned_screen_req(&stray.pane_id, None))
            .await
            .unwrap();
        let first_json: serde_json::Value =
            serde_json::from_str(&body_string(first).await).unwrap();

        // Same text again — the revision must not move.
        let repeat = app
            .clone()
            .oneshot(unassigned_screen_req(&stray.pane_id, None))
            .await
            .unwrap();
        let repeat_json: serde_json::Value =
            serde_json::from_str(&body_string(repeat).await).unwrap();
        assert_eq!(
            first_json["revision"], repeat_json["revision"],
            "unchanged unassigned pane output must keep the same revision: {first_json} vs {repeat_json}"
        );

        // The agent produces new output on its own — no input sent.
        fake.seed_scroll_pane(&stray.pane_id, "unassigned second frame", "unassigned second frame", None);

        let second = app
            .oneshot(unassigned_screen_req(&stray.pane_id, None))
            .await
            .unwrap();
        let second_json: serde_json::Value =
            serde_json::from_str(&body_string(second).await).unwrap();
        assert_ne!(
            first_json["revision"], second_json["revision"],
            "changed unassigned pane output must report a different revision: {first_json} vs {second_json}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// terminal-scroll-2: the Unassigned group's screen route gets the same
    /// `?history` branching as `terminal_screen` — with no `history` param,
    /// it must stay on today's plain `ReadSource::Recent` read, never
    /// `PaneScroller::read_history`, proven by an empty `sent_text_log`.
    #[tokio::test]
    async fn unassigned_terminal_screen_without_history_param_never_touches_pane_scroller() {
        use crate::herdr::fake::FakeHerdr;

        let dir = fresh_root("unassigned-screen-no-history");
        enable_terminal(&dir);
        enable_unassigned_group(&dir);
        let scratch = fresh_root("unassigned-screen-no-history-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let text = "unassigned live frame";
        fake.seed_scroll_pane(&stray.pane_id, text, text, Some("unassigned older frame"));

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let app = router(st);

        let resp = app
            .oneshot(unassigned_screen_req(&stray.pane_id, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["text"], serde_json::json!(mdview_core::ansi::to_html(text)), "{body}");
        assert!(
            fake.sent_text_log(&stray.pane_id).await.is_empty(),
            "an absent history param must never route through PaneScroller"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// terminal-scroll-2: `?history=2` against the Unassigned group's screen
    /// route must route through `PaneScroller::read_history` with 2 pages —
    /// same proof shape as `terminal_screen_history_param_sends_two_pageups_then_restores`.
    #[tokio::test]
    async fn unassigned_terminal_screen_history_param_sends_two_pageups_then_restores() {
        use crate::herdr::fake::FakeHerdr;
        use crate::herdr::pane_scroller::{PAGE_UP, RESTORE_BOTTOM};

        let dir = fresh_root("unassigned-screen-history-2");
        enable_terminal(&dir);
        enable_unassigned_group(&dir);
        let scratch = fresh_root("unassigned-screen-history-2-scratch");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(FakeHerdr::new());
        let stray = fake
            .agent_start("w1", Some(&stray_root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let live_bottom = "page0 (live)\n❯ ";
        fake.seed_scroll_pane(
            &stray.pane_id,
            live_bottom,
            live_bottom,
            Some("page1 further back\npage0 (live)\n❯ "),
        );
        fake.push_escape_page(
            &stray.pane_id,
            "page2 even further back\npage1 further back\npage0 (live)\n❯ ",
        );

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake.clone();
        let app = router(st);

        let resp = app
            .oneshot(Request::builder()
                .uri(format!("/_terminal/unassigned/{}/screen?history=2", stray.pane_id))
                .method("GET")
                .body(Body::empty())
                .unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let text = json["text"].as_str().unwrap();
        assert!(
            text.contains("page2 even further back"),
            "history=2 should land 2 hops back, not just 1: {text}"
        );
        assert_eq!(
            fake.sent_text_log(&stray.pane_id).await,
            vec![PAGE_UP.to_string(), PAGE_UP.to_string(), RESTORE_BOTTOM.to_string()],
            "two PageUp hops then one restore-to-bottom, in that order"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    // ---- agent-terminal-11: the Unassigned group's own guard tests ----
    //
    // toa-1 (D1/D7): the no-session and wrong-method tests these two routes
    // used to carry are retired — their subject was the auth disguise, which
    // is gone. The switch-off truth below is the only one that survives:
    // it's the only coverage left that a disabled terminal actually refuses.

    /// D12: with the D7 switch off, `/_terminal/unassigned/{pane}/screen`
    /// answers with the family's reasoned JSON 404 — no cookie, no token,
    /// nothing but the switch decides this.
    #[tokio::test]
    async fn unassigned_screen_disabled_answers_with_a_reasoned_json_404() {
        let dir = fresh_root("unassigned-screen-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let screen = app.oneshot(unassigned_screen_req("w1:p1", None)).await.unwrap();
        assert_eq!(screen.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            screen.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled unassigned screen endpoint must answer JSON, not an empty body"
        );
        let body = body_string(screen).await;
        assert!(
            body.contains("disabled"),
            "the disabled unassigned screen endpoint must name the reason: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D12: with the switch off, both unassigned write routes answer with
    /// the family's reasoned JSON 404 — no cookie, no token, nothing but the
    /// switch decides this.
    #[tokio::test]
    async fn unassigned_write_routes_disabled_answer_with_a_reasoned_json_404() {
        let dir = fresh_root("unassigned-write-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let input = app
            .clone()
            .oneshot(unassigned_input_req("w1:p1", "hi", true, None))
            .await
            .unwrap();
        assert_eq!(input.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            input.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled unassigned input endpoint must answer JSON, not an empty body"
        );
        let input_body = body_string(input).await;
        assert!(
            input_body.contains("disabled"),
            "the disabled unassigned input endpoint must name the reason: {input_body}"
        );

        let keys = app
            .oneshot(unassigned_keys_req("w1:p1", &["enter"], None))
            .await
            .unwrap();
        assert_eq!(keys.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            keys.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled unassigned keys endpoint must answer JSON, not an empty body"
        );
        let keys_body = body_string(keys).await;
        assert!(
            keys_body.contains("disabled"),
            "the disabled unassigned keys endpoint must name the reason: {keys_body}"
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
        enable_unassigned_group(&dir); // toa-4 (D9): needed for the group's routes to run.
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

        let resp = app.oneshot(unassigned_req(None)).await.unwrap();
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

        let too_many: Vec<&str> = std::iter::repeat("enter")
            .take(MAX_KEYS_PER_REQUEST + 1)
            .collect();
        let resp = app
            .oneshot(terminal_keys_req(
                &project.id,
                &started.pane_id,
                &too_many,
                None,
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
        enable_unassigned_group(&dir); // toa-4 (D9): needed for the group's routes to run.
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

        let too_many: Vec<&str> = std::iter::repeat("enter")
            .take(MAX_KEYS_PER_REQUEST + 1)
            .collect();
        let resp = app
            .oneshot(unassigned_keys_req(&stray.pane_id, &too_many, None))
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

    /// terminal-pane-scope D4: a minimal `Herdr` double whose `snapshot()`
    /// always answers a fixed, caller-built `herdr::Snapshot` verbatim —
    /// including its top-level `focused_pane_id`, which `FakeHerdr` never
    /// exposes a seam to set on an arbitrary pane (its own default seed
    /// fixes it at `w1:p1`, a pane no test project's boundary can ever
    /// accept). Every other `Herdr` method is unreachable — the page routes
    /// this double serves never call them.
    struct StaticSnapshotHerdr {
        snap: herdr::Snapshot,
    }

    #[async_trait::async_trait]
    impl herdr::Herdr for StaticSnapshotHerdr {
        async fn snapshot(&self) -> herdr::Result<herdr::Snapshot> {
            Ok(self.snap.clone())
        }
        async fn ping(&self) -> herdr::Result<herdr::ProtocolInfo> {
            unreachable!("page-selection tests never ping")
        }
        async fn read_pane(
            &self,
            _pane_id: &str,
            _source: herdr::ReadSource,
            _lines: usize,
        ) -> herdr::Result<herdr::ScreenRead> {
            unreachable!("page-selection tests never read a screen")
        }
        async fn send_input(&self, _pane_id: &str, _text: &str, _submit: bool) -> herdr::Result<()> {
            unreachable!("page-selection tests never send input")
        }
        async fn send_text(&self, _pane_id: &str, _bytes: &str) -> herdr::Result<()> {
            unreachable!("page-selection tests never send text")
        }
        async fn send_keys(&self, _pane_id: &str, _keys: &[String]) -> herdr::Result<()> {
            unreachable!("page-selection tests never send keys")
        }
        async fn tab_create(
            &self,
            _workspace_id: &str,
            _cwd: Option<&str>,
        ) -> herdr::Result<herdr::TabCreated> {
            unreachable!("page-selection tests never create a tab")
        }
        async fn agent_start(
            &self,
            _workspace_id: &str,
            _cwd: Option<&str>,
            _argv: &[String],
        ) -> herdr::Result<herdr::AgentStarted> {
            unreachable!("page-selection tests never start an agent")
        }
    }

    /// A two-pane `herdr::Snapshot`, both panes inside `path` (a single
    /// workspace/tab), with `top_focus` as the snapshot's own top-level
    /// `focused_pane_id` — the value `default_pane_id` reads to pick a
    /// bare-route default. Pane ids are `"w:p-first"`/`"w:p-second"`, in that
    /// list order, so `default_pane_id`'s "first in list order" fallback is
    /// distinguishable from its "matches the global focus" branch.
    fn two_pane_snapshot(path: &Path, top_focus: Option<&str>) -> herdr::Snapshot {
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
                focused_pane_id: Some("w:p-first".into()),
            }],
            panes: vec![
                herdr::wire::Pane {
                    pane_id: "w:p-first".into(),
                    workspace_id: "w".into(),
                    tab_id: "w:t".into(),
                    cwd: Some(p.clone()),
                    foreground_cwd: Some(p.clone()),
                },
                herdr::wire::Pane {
                    pane_id: "w:p-second".into(),
                    workspace_id: "w".into(),
                    tab_id: "w:t".into(),
                    cwd: Some(p.clone()),
                    foreground_cwd: Some(p),
                },
            ],
            focused_pane_id: top_focus.map(str::to_string),
            ..herdr::Snapshot::default()
        }
    }

    /// Case 12 (D4), happy path: a project with two panes renders a pane
    /// strip carrying two entries with two different hrefs, and each pane's
    /// own page renders exactly one `.term-screen` — never both.
    #[tokio::test]
    async fn two_panes_render_a_strip_with_two_hrefs_and_each_page_shows_one_screen() {
        let dir = fresh_root("pane-scope-two-panes-data");
        enable_terminal(&dir);
        let root = fresh_root("pane-scope-two-panes-project");
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let first = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let second = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "two-panes");
        let app = router(st);

        let bare = app.clone().oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(bare.status(), StatusCode::OK);
        let bare_body = body_string(bare).await;
        let href = |pane_id: &str| format!("/p/{}/_terminal/pane/{}", project.id, pane_id);
        assert!(
            bare_body.contains(&href(&first.pane_id)),
            "the strip must carry the first pane's own href: {bare_body}"
        );
        assert!(
            bare_body.contains(&href(&second.pane_id)),
            "the strip must carry the second pane's own href: {bare_body}"
        );
        assert_ne!(href(&first.pane_id), href(&second.pane_id), "the two hrefs must differ");

        for pane_id in [&first.pane_id, &second.pane_id] {
            let resp = app
                .clone()
                .oneshot(terminal_pane_req(&project.id, pane_id, None))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_string(resp).await;
            assert_eq!(
                body.matches("class=\"term-screen\"").count(),
                1,
                "pane {pane_id}'s own page must render exactly one screen, never two: {body}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Case 13 (D4), security: `/p/:id/_terminal/pane/:pane_id` for a pane
    /// outside the project answers the ordinary not-found page — the same
    /// refusal `terminal_screen` already makes at the data layer — and the
    /// body never names the refused pane's own id or cwd.
    #[tokio::test]
    async fn terminal_pane_page_refuses_a_pane_outside_the_project_and_never_names_it() {
        let dir = fresh_root("pane-scope-outside-data");
        enable_terminal(&dir);
        let scratch = fresh_root("pane-scope-outside-scratch");
        let root = scratch.join("owned");
        let outside = scratch.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let outside_pane = fake
            .agent_start("w1", Some(&outside.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "outside-refused");
        let app = router(st);

        let resp = app
            .clone()
            .oneshot(terminal_pane_req(&project.id, &outside_pane.pane_id, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = body_string(resp).await;
        assert!(body.contains("pane not found"), "{body}");
        assert!(
            !body.contains(&outside_pane.pane_id),
            "the refused pane's own id must never be echoed back: {body}"
        );
        assert!(
            !body.contains(&outside.to_string_lossy().into_owned()),
            "the refused pane's own cwd must never be echoed back: {body}"
        );

        // The Transcript tab's own pane-scoped route makes the same refusal.
        let transcript_resp = app
            .oneshot(transcript_pane_req(&project.id, &outside_pane.pane_id, None))
            .await
            .unwrap();
        assert_eq!(transcript_resp.status(), StatusCode::NOT_FOUND);
        let transcript_body = body_string(transcript_resp).await;
        assert!(
            !transcript_body.contains(&outside_pane.pane_id),
            "the refused pane's own id must never be echoed back on the Transcript tab either: {transcript_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Case 14 (D4), edge: the bare `/p/:id/_terminal` selects the
    /// snapshot's globally focused pane when that pane is one of this
    /// project's own, and falls back to the first pane in the project's own
    /// list order when the focus is absent or belongs to no pane of this
    /// project's.
    #[tokio::test]
    async fn bare_terminal_page_selects_the_focused_pane_when_owned_else_the_first() {
        let dir = fresh_root("pane-scope-default-data");
        enable_terminal(&dir);
        let root = fresh_root("pane-scope-default-project");

        // Focus names this project's own second pane: the second pane's own
        // page must be the one selected, not the first.
        let mut st = build_state_with_dir(&dir);
        st.herdr = std::sync::Arc::new(StaticSnapshotHerdr {
            snap: two_pane_snapshot(&root, Some("w:p-second")),
        });
        let project = register(&st, &root, "default-owned-focus");
        let app = router(st);
        let resp = app.oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("data-pane-id=\"w:p-second\""),
            "the owned, globally focused pane must be the one selected: {body}"
        );
        assert!(
            !body.contains("data-pane-id=\"w:p-first\""),
            "only the focused pane's own card may render: {body}"
        );

        // Focus names a pane that is not this project's own (or is absent
        // entirely): the fallback is the first pane in the project's own
        // list order.
        for top_focus in [Some("some-other-project:pane"), None] {
            let dir2 = fresh_root("pane-scope-default-data-b");
            enable_terminal(&dir2);
            let root2 = fresh_root("pane-scope-default-project-b");
            let mut st2 = build_state_with_dir(&dir2);
            st2.herdr = std::sync::Arc::new(StaticSnapshotHerdr {
                snap: two_pane_snapshot(&root2, top_focus),
            });
            let project2 = register(&st2, &root2, "default-fallback-first");
            let app2 = router(st2);
            let resp2 = app2.oneshot(terminal_req(&project2.id, None)).await.unwrap();
            assert_eq!(resp2.status(), StatusCode::OK);
            let body2 = body_string(resp2).await;
            assert!(
                body2.contains("data-pane-id=\"w:p-first\""),
                "with no owned focus ({top_focus:?}), the first pane in list order must be selected: {body2}"
            );
            assert!(
                !body2.contains("data-pane-id=\"w:p-second\""),
                "only the first pane's own card may render: {body2}"
            );
            std::fs::remove_dir_all(&dir2).ok();
            std::fs::remove_dir_all(&root2).ok();
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Case 4/14 continued (D4), edge: a project with no panes at all keeps
    /// today's honest empty state on the bare route — never a 404, which
    /// would be indistinguishable from an unregistered project id.
    #[tokio::test]
    async fn bare_terminal_page_with_no_panes_keeps_the_honest_empty_state() {
        let dir = fresh_root("pane-scope-empty-data");
        enable_terminal(&dir);
        let root = fresh_root("pane-scope-empty-project");
        // FakeHerdr::new()'s seeded panes all sit under
        // `/home/dev/projects/...`, which this test's own root never
        // contains — so this project's own boundary-filtered list is
        // genuinely empty, the same "no agents running" case `terminal_page`
        // has always rendered.
        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "no-panes-empty");
        let app = router(st);

        let terminal_resp = app.clone().oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(
            terminal_resp.status(),
            StatusCode::OK,
            "a project with no panes must never 404 — that is indistinguishable from an unregistered project"
        );
        let terminal_body = body_string(terminal_resp).await;
        assert!(
            terminal_body.contains("No agents are running under this project right now."),
            "an empty project must render the named empty state: {terminal_body}"
        );

        let transcript_resp = app
            .oneshot(transcript_page_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(transcript_resp.status(), StatusCode::OK);
        let transcript_body = body_string(transcript_resp).await;
        assert!(
            transcript_body.contains("No agents are running under this project right now."),
            "an empty project's Transcript tab must render the same named empty state: {transcript_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Regression (D4/D12): the D7 `terminal.enabled` switch off turns the
    /// two new pane-scoped page routes into the same disabled shape the bare
    /// routes already answer with — no cookie, no token, nothing but the
    /// switch decides this.
    #[tokio::test]
    async fn pane_scoped_pages_answer_the_disabled_shape_when_the_family_switch_is_off() {
        let dir = fresh_root("pane-scope-disabled-data");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("pane-scope-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "pane-scope-disabled");
        let app = router(st);

        let terminal_pane = app
            .clone()
            .oneshot(terminal_pane_req(&project.id, "no-such-pane", None))
            .await
            .unwrap();
        assert_eq!(terminal_pane.status(), StatusCode::NOT_FOUND);
        let terminal_pane_body = body_string(terminal_pane).await;
        assert!(
            terminal_pane_body.contains("disabled"),
            "the disabled terminal pane page must name the reason: {terminal_pane_body}"
        );

        let transcript_pane = app
            .oneshot(transcript_pane_req(&project.id, "no-such-pane", None))
            .await
            .unwrap();
        assert_eq!(transcript_pane.status(), StatusCode::NOT_FOUND);
        let transcript_pane_body = body_string(transcript_pane).await;
        assert!(
            transcript_pane_body.contains("disabled"),
            "the disabled transcript pane page must name the reason: {transcript_pane_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
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

    /// D12: with the D7 switch off, both create routes answer with the
    /// family's reasoned JSON 404 — no cookie, no token, nothing but the
    /// switch decides this.
    #[tokio::test]
    async fn terminal_create_routes_disabled_answer_with_a_reasoned_json_404()
    {
        let dir = fresh_root("terminal-create-disabled");
        // Deliberately no `enable_terminal(&dir)` call: the switch defaults off.
        let root = fresh_root("terminal-create-disabled-project");
        let st = build_state_with_dir(&dir);
        let project = register(&st, &root, "create-disabled");
        let app = router(st);

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(pane.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            pane.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled pane-create endpoint must answer JSON, not an empty body"
        );
        let pane_body = body_string(pane).await;
        assert!(
            pane_body.contains("disabled"),
            "the disabled pane-create endpoint must name the reason: {pane_body}"
        );

        let agent = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(agent.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            agent.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json",
            "the disabled agent-create endpoint must answer JSON, not an empty body"
        );
        let agent_body = body_string(agent).await;
        assert!(
            agent_body.contains("disabled"),
            "the disabled agent-create endpoint must name the reason: {agent_body}"
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

        // No presets configured at all.
        let resp = app
            .clone()
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                None,
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
                None,
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

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(pane.status(), StatusCode::CONFLICT);

        let agent = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                None,
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

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project_a.id, None))
            .await
            .unwrap();
        assert_eq!(pane.status(), StatusCode::CONFLICT);

        let agent = app
            .oneshot(create_agent_req(
                &project_a.id,
                &serde_json::json!({ "preset": "Claude" }),
                None,
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

        let pane = app
            .clone()
            .oneshot(create_pane_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(pane.status(), StatusCode::BAD_GATEWAY);

        let agent = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                None,
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

        let pane = app
            .oneshot(create_pane_req(&project.id, None))
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

        let agent = app2
            .oneshot(create_agent_req(
                &project2.id,
                &serde_json::json!({ "preset": "Claude" }),
                None,
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

        let resp = app
            .oneshot(create_pane_req(&project.id, None))
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

        let resp = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({ "preset": "Claude" }),
                None,
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

        let resp = app
            .oneshot(create_agent_req(
                &project.id,
                &serde_json::json!({
                    "preset": "Claude",
                    "argv": ["rm", "-rf", "/"],
                    "env": { "EVIL": "1" },
                    "cwd": "/etc",
                }),
                None,
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

    // ── bbp-17: the board holds up on a phone, in both themes, and from the
    // keyboard. Everything below reads the rendered board body (or, for the
    // theme pair, `views::APP_CSS` directly) rather than asserting on any
    // browser layout engine this suite does not have. ──────────────────────

    /// The board's own inline `<style>` block — everything between the first
    /// `<style>` and its matching `</style>`, so a scan below never mistakes
    /// an unrelated width (e.g. the topbar's fixed icon-button footprint in
    /// `app.css`, loaded by `<link>`, never inlined) for something the board
    /// itself declared.
    fn board_style_block(body: &str) -> &str {
        let start = body.find("<style>").expect("board body must carry its inline <style> block");
        let end = body[start..].find("</style>").expect("the inline <style> block must close");
        &body[start..start + end]
    }

    /// The largest `NNpx` value found anywhere in `css` — the responsive
    /// probe's "no fixed width wide enough to force a 375px page to scroll"
    /// half. Written by hand (no `regex` crate in this dependency tree) as a
    /// simple backward digit-walk from every `px` occurrence. A line naming
    /// `@media` is skipped: a breakpoint condition names the width a rule
    /// switches AT, never a box the page renders that wide — the narrow-
    /// screen rule above is exactly what makes that width safe to name.
    fn max_px_value(css: &str) -> u32 {
        let css = css
            .lines()
            .filter(|line| !line.contains("@media"))
            .collect::<Vec<_>>()
            .join("\n");
        let css = css.as_str();
        // Byte-level scan (never a `&str` slice at an arbitrary offset): this
        // CSS carries non-ASCII characters (an em dash in a doc comment), and
        // slicing at an offset that lands inside one of those multi-byte
        // characters would panic rather than simply fail to match "px".
        let bytes = css.as_bytes();
        let mut max = 0u32;
        let mut i = 0usize;
        while i + 1 < bytes.len() {
            if bytes[i] == b'p' && bytes[i + 1] == b'x' {
                let mut j = i;
                while j > 0 && bytes[j - 1].is_ascii_digit() {
                    j -= 1;
                }
                if j < i {
                    if let Ok(n) = std::str::from_utf8(&bytes[j..i]).unwrap().parse::<u32>() {
                        max = max.max(n);
                    }
                }
            }
            i += 1;
        }
        max
    }

    /// Every `fg-chip fg-chip--<tone>` span's visible text content, in
    /// document order — the colour probe's raw material. A tone chip with
    /// nothing but whitespace between its tags would be exactly the
    /// colour-alone defect the must-have forbids.
    fn chip_texts(body: &str) -> Vec<String> {
        const MARK: &str = "class=\"fg-chip fg-chip--";
        let mut out = Vec::new();
        let mut rest = body;
        while let Some(idx) = rest.find(MARK) {
            let after = &rest[idx..];
            let Some(gt) = after.find('>') else { break };
            let after_tag = &after[gt + 1..];
            let Some(close) = after_tag.find("</span>") else { break };
            out.push(after_tag[..close].to_string());
            rest = &after_tag[close + "</span>".len()..];
        }
        out
    }

    /// (responsive) The board declares a narrow-screen rule that collapses
    /// every multi-column grid it defines to one column, and no fixed pixel
    /// width in the board's own stylesheet is wide enough to force a 375px
    /// page to scroll sideways (every `minmax(...)` track floor here is
    /// under 375, and the narrow-screen rule replaces them with `1fr`
    /// below it regardless).
    #[tokio::test]
    async fn board_declares_narrow_screen_grid_collapse_and_no_wide_fixed_widths() {
        let root = fresh_root("responsive-collapse");
        write(&root, ".bee/cells/a.json", &cell_json("r1", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "responsive-collapse");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let style = board_style_block(&body);

        assert!(
            style.contains("@media (max-width:"),
            "the board must declare a narrow-screen breakpoint: {style}"
        );
        for grid_class in [
            ".bee-stats",
            ".bee-now-grid",
            ".bee-phase-board__cols",
            ".bee-velocity__lists",
            ".bee-panels",
            ".bee-done-grid",
            ".bee-stepper",
        ] {
            assert!(
                style.contains(grid_class),
                "the narrow-screen rule must name {grid_class} among the grids it collapses: {style}"
            );
        }
        assert!(
            style.contains("grid-template-columns: 1fr"),
            "the narrow-screen rule must collapse to a single column: {style}"
        );

        let widest = max_px_value(style);
        assert!(
            widest < 375,
            "the board's own stylesheet must declare no fixed pixel width \u{2265} 375 (found {widest}px): {style}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (responsive) The one genuinely wide container on the board — the
    /// by-phase board's column row, which can grow past a phone's width
    /// with enough phases in flight — scrolls inside itself rather than
    /// ever pushing the page wider.
    #[tokio::test]
    async fn board_wide_phase_columns_container_scrolls_within_itself() {
        let root = fresh_root("responsive-overflow");
        write(&root, ".bee/cells/a.json", &cell_json("r1", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "responsive-overflow");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let style = board_style_block(&body);

        assert!(
            style.contains(".bee-phase-board__cols") && style.contains("overflow-x: auto"),
            "the phase board's wide column row must carry its own overflow rule: {style}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (theme) `.bee/`'s dark-scheme rules are present in the shared
    /// stylesheet under BOTH the theme-agnostic axis (`html[data-scheme=
    /// "dark"]`, `contract.css`) and the atelier-specific one
    /// (`html[data-theme="atelier"][data-scheme="dark"]`, `atelier.css`) —
    /// and neither is gated behind a `prefers-color-scheme` media query
    /// anywhere in the bundle. That absence is the "beats the OS in both
    /// directions" guarantee made structural rather than incidental: the
    /// no-flash head script (`views::layout`) always resolves the saved
    /// choice (or, absent one, the OS match) to one definite `data-scheme`
    /// value before paint, and since no CSS in the bundle ever reads
    /// `prefers-color-scheme` on its own, that resolved attribute is the
    /// *only* thing either scheme's colours ever key off — an explicit
    /// `dark` choice renders dark under an OS-light preference exactly as
    /// it renders dark under OS-dark, and the same holds for an explicit
    /// `light` choice under OS-dark. Covering only one direction (e.g.
    /// asserting the dark rule exists but never checking that nothing lets
    /// the OS preference leak in on its own) is the half-fix this test
    /// exists to close.
    #[test]
    fn dark_scheme_rules_present_with_no_os_media_query_to_override_them() {
        let css = views::APP_CSS;
        assert!(
            css.contains("html[data-scheme=\"dark\"]"),
            "the theme-agnostic dark-scheme axis must be present in the bundle"
        );
        assert!(
            css.contains("html[data-theme=\"atelier\"][data-scheme=\"dark\"]"),
            "the atelier theme's own dark-scheme override must be present in the bundle"
        );
        assert!(
            !css.contains("prefers-color-scheme"),
            "no rule in the bundle may key its colours off prefers-color-scheme directly — \
             the explicit data-scheme attribute (set once, before paint, by the no-flash \
             script) must be the only thing either scheme's colours ever read, or an \
             explicit choice could stop beating the OS preference in one direction"
        );
    }

    /// (theme) The board's own rendered page carries the explicit
    /// `data-theme="atelier"` attribute and the no-flash script that always
    /// resolves to one definite `data-scheme` value (never leaving it
    /// unset for a CSS media query to fill in) — the per-page half of the
    /// same guarantee the stylesheet test above proves for the CSS itself.
    #[tokio::test]
    async fn board_page_carries_explicit_theme_attribute_and_no_flash_script() {
        let root = fresh_root("theme-attribute");
        write(&root, ".bee/cells/a.json", &cell_json("t1", "open", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "theme-attribute");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains(r#"data-theme="atelier""#),
            "the board page must carry the explicit atelier theme attribute: {body}"
        );
        assert!(
            body.contains("setAttribute('data-scheme'"),
            "the board page must carry the no-flash script that always resolves an explicit \
             data-scheme value before paint: {body}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// (colour) Every severity/state chip the board renders carries a word
    /// alongside its tone class — never a bare colour with no text a
    /// colour-blind reader could fall back on. The fixture trips several
    /// different tones at once (a stuck cell's Critical attention item, a
    /// recorded gate-bypass Warning, plus the neutral "Needs attention"
    /// count chip) so this is not just proving the one tone the happiest
    /// fixture would hit.
    #[tokio::test]
    async fn board_severity_and_state_chips_always_carry_a_word_not_color_alone() {
        let root = fresh_root("colour-chips");
        write(&root, ".bee/config.json", r#"{"gate_bypass": "total"}"#);
        write(&root, ".bee/cells/a.json", &cell_json("stuck1", "blocked", &[], "w1"));

        let st = build_state();
        let project = register(&st, &root, "colour-chips");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        let chips = chip_texts(&body);
        assert!(
            chips.len() >= 3,
            "fixture must trip several tone chips to be a real probe, found {}: {body}",
            chips.len()
        );
        for text in &chips {
            assert!(
                !text.trim().is_empty(),
                "a fg-chip carried no text alongside its colour: {body}"
            );
        }

        std::fs::remove_dir_all(&root).ok();
    }

    /// (keyboard) The finished-work disclosure is a native `<details>`/
    /// `<summary>` pair — reachable by Tab and operable with Enter/Space by
    /// the browser's own default behaviour, never a click-only `<div>` that
    /// would need a hand-rolled keyup handler to be operable at all — and
    /// the board's own stylesheet gives that `<summary>` a visible focus
    /// style, since the shared design system's generic focus-visible rule
    /// only covers a `summary` nested inside `.fg-acc`, which this one is
    /// not.
    #[tokio::test]
    async fn board_finished_disclosure_is_native_and_carries_a_visible_focus_style() {
        let root = fresh_root("keyboard-disclosure");
        write(
            &root,
            ".bee/cells/a.json",
            &timed_cell_json(
                "k1",
                "keyboard-disclosure-feature",
                "capped",
                &[],
                "w1",
                "2026-08-04T08:00:00Z",
                "2026-08-04T08:24:00Z",
            ),
        );

        let st = build_state();
        let project = register(&st, &root, "keyboard-disclosure");
        let resp = get(router(st), &format!("/p/{}/_bee", project.id)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;

        assert!(
            body.contains("<details class=\"bee-done-details\">")
                && body.contains("<summary class=\"bee-done-summary\">"),
            "the finished-work disclosure must be a native details/summary pair: {body}"
        );
        assert!(
            !body.contains("<div class=\"bee-done-summary\""),
            "the disclosure must never be a click-only div standing in for summary: {body}"
        );

        let style = board_style_block(&body);
        assert!(
            style.contains(".bee-done-summary:focus-visible"),
            "the board must give the finished-work summary its own visible focus style, \
             since the shared .fg-acc summary rule does not reach it: {style}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // ---- terminal-pane-scope: project_panes over snapshot.panes, either
    // directory, cwd first (D1/D2) ----
    //
    // Every assertion below is made on pane id, never on an agent name --
    // a shell row has no name to assert on, so an agent-name-only assertion
    // would be structurally blind to it.

    /// Case 1 (D2): a pane inside the project root with no agent at all is
    /// listed, as a shell row.
    #[tokio::test]
    async fn terminal_project_lists_a_pane_with_no_agent_as_a_shell_row() {
        let dir = fresh_root("scope-shell-row");
        enable_terminal(&dir);
        let root = fresh_root("scope-shell-row-project");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let created = fake
            .tab_create("w1", Some(&root.to_string_lossy()))
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "shell-row");
        let app = router(st);

        let resp = app.oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("data-pane-id=\"{}\"", created.pane_id)),
            "a pane with no agent must still be listed, as a shell row: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Case 2 (D1, `#[cfg(unix)]`), the worktree case measured in
    /// CONTEXT.md: a pane whose `cwd` sits outside the project root but
    /// whose `foreground_cwd` sits inside it is listed, and its screen
    /// route answers rather than refusing.
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_project_lists_a_pane_matching_only_via_foreground_cwd() {
        let dir = fresh_root("scope-fg-only-data");
        enable_terminal(&dir);
        let scratch = fresh_root("scope-fg-only-scratch");
        let root = scratch.join("project");
        let outside = scratch.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let created = fake
            .tab_create("w1", Some(&outside.to_string_lossy()))
            .await
            .unwrap();
        fake.set_pane_dirs(
            &created.pane_id,
            Some(&outside.to_string_lossy()),
            Some(&root.to_string_lossy()),
        )
        .await
        .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "fg-only");
        let app = router(st);

        let resp = app
            .clone()
            .oneshot(terminal_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("data-pane-id=\"{}\"", created.pane_id)),
            "a pane matching only via foreground_cwd must be listed: {body}"
        );

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &created.pane_id, None))
            .await
            .unwrap();
        assert_eq!(
            screen_resp.status(),
            StatusCode::OK,
            "the screen route must answer for a pane matched via foreground_cwd, not refuse it"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Case 3 (D1, the mirror direction): a pane whose `cwd` sits inside the
    /// project root but whose `foreground_cwd` sits outside it is listed --
    /// `cwd` alone is enough, on every platform, since `cwd` is tried
    /// first.
    #[tokio::test]
    async fn terminal_project_lists_a_pane_whose_foreground_cwd_is_outside_but_cwd_is_inside() {
        let dir = fresh_root("scope-cwd-only-data");
        enable_terminal(&dir);
        let scratch = fresh_root("scope-cwd-only-scratch");
        let root = scratch.join("project");
        let outside = scratch.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let created = fake
            .tab_create("w1", Some(&root.to_string_lossy()))
            .await
            .unwrap();
        fake.set_pane_dirs(
            &created.pane_id,
            Some(&root.to_string_lossy()),
            Some(&outside.to_string_lossy()),
        )
        .await
        .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "cwd-only");
        let app = router(st);

        let resp = app.oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains(&format!("data-pane-id=\"{}\"", created.pane_id)),
            "a pane whose cwd validates must be listed even though its foreground_cwd does not: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Case 4, the widening's outer edge: a pane where neither directory
    /// resolves inside the project root stays excluded, and its screen,
    /// input, and keys routes all still refuse it.
    #[tokio::test]
    async fn terminal_project_excludes_a_pane_matching_neither_directory() {
        let dir = fresh_root("scope-neither-dir-data");
        enable_terminal(&dir);
        let scratch = fresh_root("scope-neither-dir-scratch");
        let root = scratch.join("project");
        let outside = scratch.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        // agent_start sets cwd == foreground_cwd, both outside the root.
        let outside_agent = fake
            .agent_start(
                "w1",
                Some(&outside.to_string_lossy()),
                &["claude".to_string()],
            )
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "neither-dir");
        let app = router(st);

        let resp = app
            .clone()
            .oneshot(terminal_req(&project.id, None))
            .await
            .unwrap();
        let body = body_string(resp).await;
        assert!(
            !body.contains(&outside_agent.pane_id),
            "a pane matching neither directory must never be listed: {body}"
        );

        let screen_resp = app
            .clone()
            .oneshot(terminal_screen_req(
                &project.id,
                &outside_agent.pane_id,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(screen_resp.status(), StatusCode::NOT_FOUND);

        let input_resp = app
            .clone()
            .oneshot(terminal_input_req(
                &project.id,
                &outside_agent.pane_id,
                "should never land",
                Some(true),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(input_resp.status(), StatusCode::NOT_FOUND);

        let keys_resp = app
            .oneshot(terminal_keys_req(
                &project.id,
                &outside_agent.pane_id,
                &["enter"],
                None,
            ))
            .await
            .unwrap();
        assert_eq!(keys_resp.status(), StatusCode::NOT_FOUND);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Case 5 (`#[cfg(unix)]`), the security edge that turns this feature
    /// into a vulnerability if missed: a pane whose `foreground_cwd`
    /// escapes the project root by a symlink is refused, exactly as a `cwd`
    /// that does (`terminal_route_lists_only_panes_within_the_project_root_boundary`).
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_project_refuses_a_pane_whose_foreground_cwd_escapes_via_symlink() {
        let dir = fresh_root("scope-fg-symlink-data");
        enable_terminal(&dir);
        let scratch = fresh_root("scope-fg-symlink-scratch");
        let root = scratch.join("project");
        let escape_target = scratch.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&escape_target).unwrap();
        let symlink_path = root.join("escape-link");
        std::os::unix::fs::symlink(&escape_target, &symlink_path).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let created = fake
            .tab_create("w1", Some(&root.to_string_lossy()))
            .await
            .unwrap();
        // cwd absent: only foreground_cwd is consulted, and its raw path
        // sits under the root but resolves outside it.
        fake.set_pane_dirs(
            &created.pane_id,
            None,
            Some(&symlink_path.to_string_lossy()),
        )
        .await
        .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "fg-symlink");
        let app = router(st);

        let resp = app
            .clone()
            .oneshot(terminal_req(&project.id, None))
            .await
            .unwrap();
        let body = body_string(resp).await;
        assert!(
            !body.contains(&created.pane_id),
            "a foreground_cwd that escapes the root by symlink must be refused, not listed: {body}"
        );

        let screen_resp = app
            .oneshot(terminal_screen_req(&project.id, &created.pane_id, None))
            .await
            .unwrap();
        assert_eq!(
            screen_resp.status(),
            StatusCode::NOT_FOUND,
            "the screen route must refuse a pane whose only qualifying directory is a symlink escape"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Case 6, edge: a pane reporting neither `cwd` nor `foreground_cwd` is
    /// excluded from the project's list.
    #[tokio::test]
    async fn terminal_project_excludes_a_pane_reporting_neither_directory() {
        let dir = fresh_root("scope-no-dirs-data");
        enable_terminal(&dir);
        let root = fresh_root("scope-no-dirs-project");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let created = fake
            .tab_create("w1", Some(&root.to_string_lossy()))
            .await
            .unwrap();
        fake.set_pane_dirs(&created.pane_id, None, None)
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "no-dirs");
        let app = router(st);

        let resp = app.oneshot(terminal_req(&project.id, None)).await.unwrap();
        let body = body_string(resp).await;
        assert!(
            !body.contains(&created.pane_id),
            "a pane reporting neither directory must never be listed: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Case 7 (regression), the standing gap this cell leaves unchanged and
    /// pinned by pane id: a shell pane under no registered project is
    /// absent from every registered project's own list AND absent from the
    /// Unassigned group, since `unassigned_panes`' own output loop stays
    /// agent-only (`unassigned_panes`'s doc; inverting it too would newly
    /// expose every shell pane on the host through routes with no
    /// containment check of their own).
    #[tokio::test]
    async fn terminal_project_scope_shell_pane_under_no_project_is_invisible_everywhere() {
        let dir = fresh_root("scope-orphan-shell-data");
        enable_terminal(&dir);
        enable_unassigned_group(&dir);
        let scratch = fresh_root("scope-orphan-shell-scratch");
        let owned_root = scratch.join("owned");
        let stray_root = scratch.join("stray");
        std::fs::create_dir_all(&owned_root).unwrap();
        std::fs::create_dir_all(&stray_root).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let stray = fake
            .tab_create("w1", Some(&stray_root.to_string_lossy()))
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &owned_root, "orphan-shell-owner");
        let app = router(st);

        let project_resp = app
            .clone()
            .oneshot(terminal_req(&project.id, None))
            .await
            .unwrap();
        let project_body = body_string(project_resp).await;
        assert!(
            !project_body.contains(&stray.pane_id),
            "the stray shell pane must not leak into an unrelated project's own list: {project_body}"
        );

        let unassigned_resp = app.oneshot(unassigned_req(None)).await.unwrap();
        assert_eq!(unassigned_resp.status(), StatusCode::OK);
        let unassigned_body = body_string(unassigned_resp).await;
        assert!(
            !unassigned_body.contains(&stray.pane_id),
            "a shell pane under no registered project is the standing, pinned gap -- it must \
             stay invisible to the Unassigned group too, not newly appear there: {unassigned_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// Case 8 (`#[cfg(unix)]`): the precedence rule that keeps an existing
    /// pane's transcript from silently re-keying. A pane matched only via
    /// `foreground_cwd` keys its transcript read on that matched path,
    /// answering `available: false` when nothing has been written there
    /// (the honest empty state, not an error) -- and a pane whose `cwd`
    /// validates keys its transcript on `cwd`, even when `foreground_cwd`
    /// also validates.
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_transcript_keys_on_cwd_first_and_on_foreground_cwd_only_when_cwd_does_not_validate(
    ) {
        let dir = fresh_root("scope-transcript-precedence-data");
        enable_terminal(&dir);
        let scratch = fresh_root("scope-transcript-precedence-scratch");
        let root = scratch.join("project");
        let outside = scratch.join("outside");
        let inner_cwd = root.join("launched-here");
        let inner_fg = root.join("live-here");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&inner_cwd).unwrap();
        std::fs::create_dir_all(&inner_fg).unwrap();
        let transcript_root = fresh_root("scope-transcript-precedence-claude");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());

        // Sub-case: matched only via foreground_cwd -- nothing is ever
        // written at that path, so the transcript route must answer
        // available:false rather than 404 (membership already passed).
        let fg_only = fake
            .tab_create("w1", Some(&outside.to_string_lossy()))
            .await
            .unwrap();
        fake.set_pane_dirs(
            &fg_only.pane_id,
            Some(&outside.to_string_lossy()),
            Some(&inner_fg.to_string_lossy()),
        )
        .await
        .unwrap();

        // Sub-case: both directories validate -- the transcript is written
        // only at the cwd path, proving cwd wins the precedence.
        let both_match = fake
            .tab_create("w1", Some(&inner_cwd.to_string_lossy()))
            .await
            .unwrap();
        fake.set_pane_dirs(
            &both_match.pane_id,
            Some(&inner_cwd.to_string_lossy()),
            Some(&inner_fg.to_string_lossy()),
        )
        .await
        .unwrap();
        let cwd_canonical = canonical_cwd(&inner_cwd);
        write_transcript(
            &transcript_root,
            &cwd_canonical,
            "s1",
            "{\"type\":\"user\",\"message\":{\"content\":\"launched here\"}}\n",
        );

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        st.transcript_root = Some(transcript_root.clone());
        let project = register(&st, &root, "transcript-precedence");
        let app = router(st);

        let fg_resp = app
            .clone()
            .oneshot(terminal_transcript_req(
                &project.id,
                &fg_only.pane_id,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(fg_resp.status(), StatusCode::OK);
        let fg_json: serde_json::Value = serde_json::from_str(&body_string(fg_resp).await).unwrap();
        assert_eq!(
            fg_json["available"],
            serde_json::json!(false),
            "a pane matched only via foreground_cwd, with nothing written there, must answer \
             the honest empty state: {fg_json}"
        );

        let both_resp = app
            .oneshot(terminal_transcript_req(
                &project.id,
                &both_match.pane_id,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(both_resp.status(), StatusCode::OK);
        let both_json: serde_json::Value =
            serde_json::from_str(&body_string(both_resp).await).unwrap();
        assert_eq!(
            both_json["available"],
            serde_json::json!(true),
            "a pane whose cwd validates must key its transcript on cwd, even though \
             foreground_cwd also validates: {both_json}"
        );
        let lines: Vec<String> = both_json["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(lines, vec!["» launched here".to_string()], "{both_json}");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
        std::fs::remove_dir_all(&transcript_root).ok();
    }

    /// Case 9 (D1): a pane can legitimately qualify for two registered
    /// projects at once (a parent repo and its worktree) -- it is listed by
    /// both, and each project's own screen route serves it under its own
    /// boundary, not by borrowing the other's.
    #[tokio::test]
    async fn terminal_project_a_pane_qualifying_for_two_projects_is_listed_by_both() {
        let dir = fresh_root("scope-two-projects-data");
        enable_terminal(&dir);
        let scratch = fresh_root("scope-two-projects-scratch");
        let parent_root = scratch.join("parent");
        let child_root = parent_root.join("child-worktree");
        std::fs::create_dir_all(&child_root).unwrap();

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let shared = fake
            .agent_start(
                "w1",
                Some(&child_root.to_string_lossy()),
                &["claude".to_string()],
            )
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let parent = register(&st, &parent_root, "two-projects-parent");
        let child = register(&st, &child_root, "two-projects-child");
        let app = router(st);

        let parent_resp = app
            .clone()
            .oneshot(terminal_req(&parent.id, None))
            .await
            .unwrap();
        let parent_body = body_string(parent_resp).await;
        assert!(
            parent_body.contains(&format!("data-pane-id=\"{}\"", shared.pane_id)),
            "the shared pane is missing from the parent project's own list: {parent_body}"
        );

        let child_resp = app
            .clone()
            .oneshot(terminal_req(&child.id, None))
            .await
            .unwrap();
        let child_body = body_string(child_resp).await;
        assert!(
            child_body.contains(&format!("data-pane-id=\"{}\"", shared.pane_id)),
            "the shared pane is missing from the child project's own list: {child_body}"
        );

        let parent_screen = app
            .clone()
            .oneshot(terminal_screen_req(&parent.id, &shared.pane_id, None))
            .await
            .unwrap();
        assert_eq!(parent_screen.status(), StatusCode::OK);

        let child_screen = app
            .oneshot(terminal_screen_req(&child.id, &shared.pane_id, None))
            .await
            .unwrap();
        assert_eq!(child_screen.status(), StatusCode::OK);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    // ---- terminal-pane-scope cell 2: a card names its workspace, its tab,
    // and a status that admits having no agent (D3) ----

    /// Case 10 (D3): a card renders its workspace label and its tab label
    /// together as its identity, on the Terminal tab and the Transcript tab
    /// alike -- CONTEXT.md's own gap: "Today's card shows bare status text
    /// with no workspace or tab identity."
    #[tokio::test]
    async fn terminal_and_transcript_cards_name_the_panes_workspace_and_tab() {
        let dir = fresh_root("scope-identity-data");
        enable_terminal(&dir);
        let root = fresh_root("scope-identity-project");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        fake.agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "identity");
        let app = router(st);

        let terminal_resp = app
            .clone()
            .oneshot(terminal_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(terminal_resp.status(), StatusCode::OK);
        let terminal_body = body_string(terminal_resp).await;
        assert!(
            terminal_body.contains("frontend-app · main"),
            "the terminal card must name its workspace and tab together: {terminal_body}"
        );

        let transcript_resp = app
            .oneshot(transcript_page_req(&project.id, None))
            .await
            .unwrap();
        assert_eq!(transcript_resp.status(), StatusCode::OK);
        let transcript_body = body_string(transcript_resp).await;
        assert!(
            transcript_body.contains("frontend-app · main"),
            "the transcript card must name its workspace and tab together: {transcript_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Case 10, continued (D3): a working, an idle, a done and a blocked
    /// pane each render a status pill whose class differs from the other
    /// three -- the six states this cell owns map onto `.fg-status`'s three
    /// tone modifiers (`components.css:145-151`) with no CSS file touched.
    ///
    /// terminal-pane-scope D4: `terminal_page` now renders exactly one
    /// pane's card, so each status is checked on that pane's own
    /// `/p/:id/_terminal/pane/:pane_id` page rather than by slicing four
    /// cards out of one shared body.
    #[tokio::test]
    async fn terminal_page_renders_a_distinct_status_pill_class_per_status() {
        let dir = fresh_root("scope-status-pill-data");
        enable_terminal(&dir);
        let root = fresh_root("scope-status-pill-project");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let working = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.set_status(&working.pane_id, herdr::AgentStatus::Working)
            .await
            .unwrap();
        let idle = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.set_status(&idle.pane_id, herdr::AgentStatus::Idle)
            .await
            .unwrap();
        let done = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.set_status(&done.pane_id, herdr::AgentStatus::Done)
            .await
            .unwrap();
        let blocked = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.set_status(&blocked.pane_id, herdr::AgentStatus::Blocked)
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "status-pill");
        let app = router(st);

        async fn pane_page(app: Router, project_id: &str, pane_id: &str) -> String {
            let resp = app
                .oneshot(terminal_pane_req(project_id, pane_id, None))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_string(resp).await
        }
        // The pill lives on the pane's tab, the only place a pane's identity
        // is printed now that the card carries no heading of its own.
        let card = |body: &str, pane_id: &str| -> String {
            let start = body
                .find(&format!("/pane/{pane_id}\""))
                .unwrap_or_else(|| panic!("no tab for {pane_id}: {body}"));
            let end = body[start..]
                .find("</a>")
                .map(|i| start + i)
                .unwrap_or(body.len());
            body[start..end].to_string()
        };

        let working_body = pane_page(app.clone(), &project.id, &working.pane_id).await;
        assert!(
            card(&working_body, &working.pane_id).contains("class=\"fg-status fg-status--warn\""),
            "working must render the warn pill: {working_body}"
        );
        let blocked_body = pane_page(app.clone(), &project.id, &blocked.pane_id).await;
        assert!(
            card(&blocked_body, &blocked.pane_id).contains("class=\"fg-status fg-status--blocked\""),
            "blocked must render the blocked pill: {blocked_body}"
        );
        let done_body = pane_page(app.clone(), &project.id, &done.pane_id).await;
        assert!(
            card(&done_body, &done.pane_id).contains("class=\"fg-status fg-status--ready\""),
            "done must render the ready pill: {done_body}"
        );
        let idle_body = pane_page(app, &project.id, &idle.pane_id).await;
        let idle_card = card(&idle_body, &idle.pane_id);
        assert!(
            idle_card.contains("class=\"fg-status\">"),
            "idle must render the neutral, unmodified pill: {idle_body}"
        );
        assert!(
            !idle_card.contains("fg-status--"),
            "idle must not borrow another status's modifier: {idle_body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Case 10, edge (D3): `AgentStatus::Unknown` -- any status value herdr
    /// adds later that this app does not recognize (`wire.rs:27`) -- renders
    /// the same neutral, unmodified `.fg-status` pill idle does, never
    /// borrowing `--ready`/`--warn`/`--blocked` from a state it is not.
    #[tokio::test]
    async fn terminal_page_an_unknown_status_renders_the_neutral_pill() {
        let dir = fresh_root("scope-status-unknown-data");
        enable_terminal(&dir);
        let root = fresh_root("scope-status-unknown-project");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let started = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        fake.set_status(&started.pane_id, herdr::AgentStatus::Unknown)
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "status-unknown");
        let app = router(st);

        let resp = app.oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("class=\"fg-status\">"),
            "an unknown status must render the neutral pill: {body}"
        );
        assert!(
            !body.contains("fg-status--"),
            "an unknown status must not borrow another state's colour: {body}"
        );
        assert!(
            body.contains(">unknown<"),
            "the pill's own text must still name the state: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Case 11 (D2/D3): a shell row (no agent) renders no agent kind and
    /// claims no status it does not have -- both the meta text and the
    /// status pill's own text read "shell" rather than either being blank
    /// or borrowing `claude`/`codex`/an `AgentStatus`. That identity now
    /// lives on the pane tab strip, the only place it is printed: the card
    /// itself carries no heading, so this reads the strip, not the card.
    #[tokio::test]
    async fn terminal_page_a_shell_row_names_itself_a_shell() {
        let dir = fresh_root("scope-shell-identity-data");
        enable_terminal(&dir);
        let root = fresh_root("scope-shell-identity-project");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let created = fake
            .tab_create("w1", Some(&root.to_string_lossy()))
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "shell-identity");
        let app = router(st);

        let resp = app.oneshot(terminal_req(&project.id, None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let start = body
            .find("class=\"pane-strip\"")
            .unwrap_or_else(|| panic!("no pane strip on the terminal page: {body}"));
        let end = body[start..]
            .find("</nav>")
            .map(|i| start + i)
            .unwrap_or(body.len());
        let strip = &body[start..end];

        assert!(
            strip.contains(&format!("/pane/{}", created.pane_id)),
            "no tab for the shell pane: {body}"
        );
        assert!(
            strip.contains("class=\"term-pane__meta\">shell<"),
            "a shell row must name itself a shell, not an agent kind: {body}"
        );
        assert!(
            !strip.contains("claude") && !strip.contains("codex"),
            "a shell row must claim no agent kind: {body}"
        );
        assert!(
            strip.contains("class=\"fg-status\">") && strip.contains(">shell<"),
            "a shell row's status pill must name it a shell, not claim a status it doesn't have: {body}"
        );
        assert!(
            body.contains(&format!(
                "class=\"fg-card term-pane\" data-pane-id=\"{}\"",
                created.pane_id
            )),
            "no card for the shell pane: {body}"
        );
        assert!(
            !body.contains("term-pane__head"),
            "a pane card must carry no heading of its own: {body}"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// Regression (cell 2 must not touch any existing control): the reply
    /// form, the key buttons and the scroll buttons stay on every terminal
    /// card, including a shell row with no agent to reply to or send keys
    /// at, and the Transcript tab still carries no `.term-screen` viewport.
    ///
    /// terminal-pane-scope D4: `terminal_page`/`transcript_page` now render
    /// exactly one pane, so each pane's controls are checked on its own
    /// `/pane/:pane_id` page rather than on one page shared by both panes.
    #[tokio::test]
    async fn terminal_and_transcript_cards_keep_every_existing_control_including_on_a_shell_row() {
        let dir = fresh_root("scope-controls-survive-data");
        enable_terminal(&dir);
        let root = fresh_root("scope-controls-survive-project");

        let fake = std::sync::Arc::new(crate::herdr::fake::FakeHerdr::new());
        let agent_pane = fake
            .agent_start("w1", Some(&root.to_string_lossy()), &["claude".to_string()])
            .await
            .unwrap();
        let shell_pane = fake
            .tab_create("w1", Some(&root.to_string_lossy()))
            .await
            .unwrap();

        let mut st = build_state_with_dir(&dir);
        st.herdr = fake;
        let project = register(&st, &root, "controls-survive");
        let app = router(st);

        for pane_id in [&agent_pane.pane_id, &shell_pane.pane_id] {
            let terminal_resp = app
                .clone()
                .oneshot(terminal_pane_req(&project.id, pane_id, None))
                .await
                .unwrap();
            assert_eq!(terminal_resp.status(), StatusCode::OK);
            let terminal_body = body_string(terminal_resp).await;
            assert!(
                terminal_body.contains(&format!("class=\"term-reply\" data-pane-id=\"{pane_id}\"")),
                "the reply form must survive on every terminal card, including a shell row ({pane_id}): {terminal_body}"
            );
            assert!(
                terminal_body.contains(&format!("class=\"term-keys\" data-pane-id=\"{pane_id}\"")),
                "the key buttons must survive on every terminal card, including a shell row ({pane_id}): {terminal_body}"
            );
            assert!(
                terminal_body.contains(&format!("class=\"term-scroll\" data-pane-id=\"{pane_id}\"")),
                "the scroll buttons must survive on every terminal card, including a shell row ({pane_id}): {terminal_body}"
            );
        }

        for pane_id in [&agent_pane.pane_id, &shell_pane.pane_id] {
            let transcript_resp = app
                .clone()
                .oneshot(transcript_pane_req(&project.id, pane_id, None))
                .await
                .unwrap();
            assert_eq!(transcript_resp.status(), StatusCode::OK);
            let transcript_body = body_string(transcript_resp).await;
            assert!(
                !transcript_body.contains("class=\"term-screen\""),
                "the transcript tab must still carry no screen element: {transcript_body}"
            );
            assert!(
                transcript_body.contains(&format!(
                    "class=\"term-transcript\" data-pane-id=\"{pane_id}\""
                )),
                "every pane, including a shell row, must still get a transcript viewport ({pane_id}): {transcript_body}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    // ── projects-home-2: POST /api/projects/register (D7/D8/D9a/D10) ──────

    /// D7/D8: a valid absolute directory registers, redirects 303 to `/`,
    /// and the derived name is the directory's own name — `register_project`
    /// passes `None` to `Engine::register` rather than a name field the
    /// form never carries.
    #[tokio::test]
    async fn register_project_happy_path_registers_and_redirects() {
        let dir = fresh_root("register-happy-path");
        let scratch = fresh_root("register-happy-path-scratch");
        let root = scratch.join("newproj");
        std::fs::create_dir_all(&root).unwrap();
        write(&root, "README.md", "# New Project");

        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let app = router(st);

        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "path={}",
                urlencoding_lite(&root.to_string_lossy())
            )))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SEE_OTHER,
            "a valid path must redirect 303"
        );
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/",
            "a successful registration must redirect to /"
        );

        let canonical = std::fs::canonicalize(&root).unwrap();
        let projects = engine.list_projects().unwrap();
        let found = projects
            .iter()
            .find(|p| p.root_path == canonical)
            .unwrap_or_else(|| panic!("the new root was not registered: {projects:?}"));
        assert_eq!(
            found.name,
            canonical.file_name().unwrap().to_string_lossy(),
            "the derived name must be the directory's own name"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D9a: an ordinary path outside every existing project root DOES
    /// register — there is no allow-list anywhere in this route. This is
    /// D9a's openness pinned as deliberate: a later reader must never mistake
    /// the absence of a guard here for an oversight.
    #[tokio::test]
    async fn register_project_registers_an_ordinary_path_with_no_allow_list() {
        let dir = fresh_root("register-no-allowlist");
        let scratch = fresh_root("register-no-allowlist-scratch");
        // Nothing under `scratch` is registered anywhere, and no allowed-roots
        // list constrains this route (D9a) — an arbitrary ordinary directory
        // must still register.
        let root = scratch.join("unrelated").join("elsewhere");
        std::fs::create_dir_all(&root).unwrap();

        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let app = router(st);

        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "path={}",
                urlencoding_lite(&root.to_string_lossy())
            )))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);

        let canonical = std::fs::canonicalize(&root).unwrap();
        assert!(
            engine
                .list_projects()
                .unwrap()
                .iter()
                .any(|p| p.root_path == canonical),
            "a path outside every registered root must still register — D9a keeps the route open"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D10: an empty path, a relative path, and an absolute path carrying a
    /// raw `..` component are each refused before any filesystem check that
    /// could otherwise let the `..` resolve away into an accepted root.
    /// `engine.list_projects().len()` is asserted unchanged after every one.
    #[tokio::test]
    async fn register_project_refuses_empty_relative_and_dotdot_paths() {
        let dir = fresh_root("register-invalid-path");
        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st);

        for raw in ["", "relative/path", "/tmp/../tmp"] {
            let req = Request::builder()
                .method("POST")
                .uri("/api/projects/register")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("path={}", urlencoding_lite(raw))))
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::SEE_OTHER);
            let location = resp
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap();
            assert_eq!(
                location, "/?register_error=invalid_path",
                "path {raw:?} must be refused as invalid_path, got redirect to {location}"
            );
        }

        assert_eq!(
            engine.list_projects().unwrap().len(),
            before,
            "none of the invalid paths may register a project"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D10: a path to an existing regular file (not a directory) is refused,
    /// distinctly from a missing path.
    #[tokio::test]
    async fn register_project_refuses_a_regular_file_as_not_a_directory() {
        let dir = fresh_root("register-not-directory");
        let scratch = fresh_root("register-not-directory-scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        write(&scratch, "plain-file.txt", "just a file");
        let file_path = scratch.join("plain-file.txt");

        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st);

        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "path={}",
                urlencoding_lite(&file_path.to_string_lossy())
            )))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/?register_error=not_directory"
        );
        assert_eq!(engine.list_projects().unwrap().len(), before);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D9a: a root sitting on `paths_boundary::hard_deny_list` (`/etc` —
    /// `$HOME/.ssh` is the example CONTEXT.md names, but it may not exist in
    /// a CI sandbox; `/etc` is the same deny-listed class of directory and
    /// is guaranteed to exist) is refused, and nothing is indexed. This
    /// proves the route goes through `Boundary::new` rather than never
    /// consulting the deny list at all — the gap the plan's P1 names.
    #[tokio::test]
    async fn register_project_refuses_a_root_on_the_hard_deny_list() {
        let dir = fresh_root("register-deny-list");
        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st);

        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("path=%2Fetc"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/?register_error=denied",
            "a deny-listed root must be refused"
        );
        assert_eq!(
            engine.list_projects().unwrap().len(),
            before,
            "a deny-listed root must never register, and nothing gets indexed"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D9b: `Boundary::new` alone only answers "is this root inside a denied
    /// root", so `POST path=$HOME` passed that gate and went on to index
    /// (and later serve, unauthenticated) markdown under `~/.ssh`, `~/.aws`
    /// and `~/.gnupg` — the credential-directory case D9a exists to close.
    /// `/` and `$HOME` both *contain* a hard-deny-listed directory without
    /// sitting inside one, which is exactly the direction
    /// `paths_boundary::is_denied_root` adds.
    #[tokio::test]
    async fn register_project_refuses_a_root_that_contains_a_denied_directory() {
        let dir = fresh_root("register-deny-containment");
        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st.clone());

        // "/" contains /etc (and every other hard-deny-listed root) — refused
        // regardless of what $HOME happens to be in this environment.
        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("path=%2F"))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/?register_error=denied",
            "/ contains a denied directory and must be refused"
        );

        // $HOME contains $HOME/.ssh, $HOME/.aws, etc — refused even though
        // $HOME itself is never named on the deny list.
        if let Some(home) = std::env::var_os("HOME") {
            let req = Request::builder()
                .method("POST")
                .uri("/api/projects/register")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "path={}",
                    urlencoding_lite(&home.to_string_lossy())
                )))
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                resp.headers().get(header::LOCATION).unwrap(),
                "/?register_error=denied",
                "$HOME contains a denied directory and must be refused"
            );
        }

        assert_eq!(
            engine.list_projects().unwrap().len(),
            before,
            "neither / nor $HOME may register, and nothing gets indexed"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D9b's containment refusal is not an allow-list in disguise: an
    /// ordinary directory under the home directory — sharing no path
    /// component with any hard-deny-listed entry beyond `$HOME` itself —
    /// still registers.
    #[tokio::test]
    async fn register_project_still_registers_an_ordinary_directory_under_home() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let dir = fresh_root("register-under-home");
        let root = std::path::PathBuf::from(&home)
            .join("mdview-bee-test-register-under-home-projects-home-3");
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();

        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let app = router(st);

        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "path={}",
                urlencoding_lite(&root.to_string_lossy())
            )))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/",
            "an ordinary directory under home must still register — D9b denies containment, not the whole home tree"
        );
        let canonical = std::fs::canonicalize(&root).unwrap();
        assert!(
            engine
                .list_projects()
                .unwrap()
                .iter()
                .any(|p| p.root_path == canonical),
            "the ordinary under-home directory must actually be registered"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// D10: re-registering an already-registered root is refused with the
    /// duplicate code, and so is its trailing-slash form — a raw-string
    /// comparison against the stored `root_path` would miss the
    /// trailing-slash variant and fall through to a silent
    /// `ensure_project` success, which is exactly what D10 forbids.
    #[tokio::test]
    async fn register_project_refuses_a_duplicate_root_and_its_trailing_slash_form() {
        let dir = fresh_root("register-duplicate");
        let scratch = fresh_root("register-duplicate-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();

        let st = build_state_with_dir(&dir);
        register(&st, &root, "demo");
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st);

        for raw in [
            root.to_string_lossy().into_owned(),
            format!("{}/", root.to_string_lossy()),
        ] {
            let req = Request::builder()
                .method("POST")
                .uri("/api/projects/register")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("path={}", urlencoding_lite(&raw))))
                .unwrap();
            let resp = app.clone().oneshot(req).await.unwrap();
            assert_eq!(
                resp.headers().get(header::LOCATION).unwrap(),
                "/?register_error=duplicate",
                "{raw:?} must be refused as a duplicate"
            );
        }

        assert_eq!(
            engine.list_projects().unwrap().len(),
            before,
            "no duplicate submission may change the project count"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D10: a symlink to an already-registered root is also a duplicate — it
    /// canonicalizes to the same path, so only comparing the canonical form
    /// (never the raw submitted string) catches it.
    #[cfg(unix)]
    #[tokio::test]
    async fn register_project_refuses_a_symlink_to_an_already_registered_root() {
        let dir = fresh_root("register-duplicate-symlink");
        let scratch = fresh_root("register-duplicate-symlink-scratch");
        let root = scratch.join("demo");
        std::fs::create_dir_all(&root).unwrap();
        let link = scratch.join("demo-link");
        std::os::unix::fs::symlink(&root, &link).unwrap();

        let st = build_state_with_dir(&dir);
        register(&st, &root, "demo");
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st);

        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "path={}",
                urlencoding_lite(&link.to_string_lossy())
            )))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/?register_error=duplicate",
            "a symlink to an already-registered root must be refused as a duplicate"
        );
        assert_eq!(engine.list_projects().unwrap().len(), before);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D9a: a tree whose markdown count exceeds `REGISTER_MAX_MARKDOWN_FILES`
    /// is refused before `Engine::register` ever runs, registering nothing —
    /// the pre-flight this cell adds specifically to close the P1 unbounded
    /// walk (`indexer.rs:88-107` via `engine.rs:72-77`).
    #[tokio::test]
    async fn register_project_refuses_a_tree_over_the_markdown_file_cap() {
        let dir = fresh_root("register-too-large");
        let scratch = fresh_root("register-too-large-scratch");
        let root = scratch.join("huge");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..(REGISTER_MAX_MARKDOWN_FILES + 1) {
            write(&root, &format!("f{i}.md"), "# x");
        }

        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st);

        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!(
                "path={}",
                urlencoding_lite(&root.to_string_lossy())
            )))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.headers().get(header::LOCATION).unwrap(),
            "/?register_error=too_large",
            "a tree over the markdown-file cap must be refused"
        );
        assert_eq!(
            engine.list_projects().unwrap().len(),
            before,
            "an oversized tree must register nothing"
        );

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&scratch).ok();
    }

    /// D10/P1: the rejected path string must never appear in the response
    /// body of the page a refusal redirects to — a fixed code carries the
    /// same information without anything to inject on this unauthenticated
    /// route.
    #[tokio::test]
    async fn register_project_never_echoes_the_submitted_path_into_the_page() {
        let dir = fresh_root("register-no-echo");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let marker = "UNIQUE-MARKER-should-never-render-4f8c";
        let raw = format!("/definitely/not/real/{marker}");
        let req = Request::builder()
            .method("POST")
            .uri("/api/projects/register")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("path={}", urlencoding_lite(&raw))))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let location = resp
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(location, "/?register_error=not_found");

        let page = body_string(get(app, &location).await).await;
        assert!(
            !page.contains(marker),
            "the submitted path must never be echoed into the refusal page: {page}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// toa-1: mounted with its one true method (`post`) — a `GET` never
    /// reaches the handler and registers nothing, the built-in axum 405 the
    /// same family of route already relies on
    /// (`a_get_carrying_switch_values_in_its_query_changes_no_switch`).
    #[tokio::test]
    async fn get_register_project_route_is_405_and_registers_nothing() {
        let dir = fresh_root("register-get-405");
        let st = build_state_with_dir(&dir);
        let engine = st.engine.clone();
        let before = engine.list_projects().unwrap().len();
        let app = router(st);

        let req = Request::builder()
            .method("GET")
            .uri("/api/projects/register?path=%2Ftmp")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "GET must never reach the register handler"
        );
        assert_eq!(engine.list_projects().unwrap().len(), before);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D10: `#[derive(Deserialize)]`'s generated struct visitor refuses a
    /// repeated query key outright, and nothing about a redirect target rules
    /// one out — a bookmarked or hand-edited URL, or a client that appends
    /// rather than replaces a query parameter, can send
    /// `register_error` twice. That must render the Projects page, never
    /// turn `/` itself into a 400.
    #[tokio::test]
    async fn duplicated_register_error_query_key_still_renders_the_page() {
        let dir = fresh_root("register-flag-duplicate-key");
        let st = build_state_with_dir(&dir);
        let app = router(st);

        let resp = get(app, "/?register_error=a&register_error=b").await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a duplicated register_error query key must still render the Projects page"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
