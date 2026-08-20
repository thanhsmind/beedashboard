//! Minimal MCP server over stdio (newline-delimited JSON-RPC 2.0).
//! Exposes the agent-facing query surface (PRD §5.5): the original write-side
//! `waggledance_view_file`, three read-only query tools —
//! `waggledance_search`, `waggledance_projects`, `waggledance_ask_state`
//! (mcp-query-surface D3) — and the orchestrator-dispatch write surface
//! (`docs/history/orchestrator-dispatch/plan.md` D2): `waggledance_dispatch`,
//! `waggledance_await`, `waggledance_runs`. Hand-rolled to avoid a heavy SDK
//! dependency; the protocol surface here is intentionally small.

use crate::herdr::{socket::SocketHerdr, Herdr};
use crate::orchestrate;
use crate::runtime;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;
use waggledance_core::bee;
use waggledance_core::config::registry_db_path;
use waggledance_core::domain::{Project, Run};
use waggledance_core::notify_store::NotifyStore;
use waggledance_core::{Config, Engine, Error, SqliteStore};

/// Default `waggledance_search` hit cap when the caller does not pass `limit`.
const DEFAULT_SEARCH_LIMIT: usize = 10;

/// Default `waggledance_runs` row cap per project when the caller does not
/// pass `limit` (no `limit` field is exposed on the tool at all — every
/// listed project is capped the same way).
const RUNS_LIST_LIMIT: usize = 100;

const PROTOCOL_VERSION: &str = "2024-11-05";

/// The owned tokio runtime + herdr socket client the dispatch-family tools
/// need — `mcp.rs`'s stdio loop is otherwise sync with no herdr client at
/// all (module doc). Built lazily on the first `waggledance_dispatch`/
/// `waggledance_await` call (`orchestration_handle`) so a session that never
/// touches orchestration never pays for either; `waggledance_runs` needs
/// neither and never triggers this.
struct Orchestration {
    runtime: tokio::runtime::Runtime,
    herdr: SocketHerdr,
    notify_store: Option<NotifyStore>,
}

impl Orchestration {
    fn init(cfg: &waggledance_core::config::TerminalConfig) -> std::result::Result<Self, String> {
        Self::init_with_override(cfg, None)
    }

    fn init_with_override(
        cfg: &waggledance_core::config::TerminalConfig,
        override_dir: Option<&Path>,
    ) -> std::result::Result<Self, String> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| format!("failed to start the orchestration runtime: {e}"))?;
        // Mirrors `server.rs`'s own best-effort fallback: an unresolvable
        // default socket path never blocks startup, it just means every
        // herdr call below fails with a named "unavailable" error instead
        // of a crash.
        let socket_path = crate::herdr::socket::default_socket_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent/herdr.sock"));
        let notify_store = open_notify_store(cfg, override_dir);
        Ok(Orchestration {
            runtime,
            herdr: SocketHerdr::new(socket_path),
            notify_store,
        })
    }
}

/// D7/D9 outbox for the MCP stdio process (dbn-4): Orchestration opens its own
/// NotifyStore against the SAME database file the server uses.
/// Per D6 the opt-in switch still governs: open the store and pass Some only
/// when the notify switch is enabled in the configuration, and None when it is off.
/// A store that fails to open falls back to None and logs one warning, matching
/// the degrade-rather-than-fail shape server.rs uses.
fn open_notify_store(
    cfg: &waggledance_core::config::TerminalConfig,
    override_dir: Option<&Path>,
) -> Option<NotifyStore> {
    if !cfg.notify_enabled {
        return None;
    }
    let notify_store_path = waggledance_core::config::notify_store_path_override(override_dir);
    NotifyStore::open(&notify_store_path)
        .map_err(|e| {
            tracing::warn!("notify outbox open failed ({e}); notifications disabled");
            e
        })
        .ok()
}

/// Lazily build (or reuse) the shared [`Orchestration`] handle.
fn orchestration_handle<'a>(
    slot: &'a mut Option<Orchestration>,
    cfg: &waggledance_core::config::TerminalConfig,
) -> std::result::Result<&'a Orchestration, String> {
    if slot.is_none() {
        *slot = Some(Orchestration::init(cfg)?);
    }
    Ok(slot.as_ref().expect("just initialized above"))
}

pub fn run() -> Result<()> {
    let engine = Engine::new(SqliteStore::open(&registry_db_path())?, Config::load());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut orchestration: Option<Orchestration> = None;

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Notifications have no id and expect no response.
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => Some(ok(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "waggledance", "version": env!("CARGO_PKG_VERSION") }
                }),
            )),
            "tools/list" => Some(ok(
                id,
                json!({
                    "tools": [
                        view_file_schema(),
                        search_schema(),
                        projects_schema(),
                        ask_state_schema(),
                        dispatch_schema(),
                        await_schema(),
                        runs_schema()
                    ]
                }),
            )),
            "tools/call" => Some(handle_tool_call(id, &engine, &mut orchestration, &req)),
            "ping" => Some(ok(id, json!({}))),
            _ if id.is_some() => Some(err(id, -32601, "method not found")),
            _ => None, // notification
        };

        if let Some(resp) = response {
            writeln!(stdout, "{resp}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn view_file_schema() -> Value {
    json!({
        "name": "waggledance_view_file",
        "description": "Make a markdown file viewable in the browser and return its URL. \
    Auto-registers the project on first use and indexes the file immediately. \
    Pass the project root and the file path relative to that root.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project_root": { "type": "string", "description": "Absolute path to the project root" },
                "relative_path": { "type": "string", "description": "Markdown file path relative to project_root" }
            },
            "required": ["project_root", "relative_path"]
        }
    })
}

fn search_schema() -> Value {
    json!({
        "name": "waggledance_search",
        "description": "Full-text search across every registered project's indexed markdown \
    (or one project, when `project` is given). Re-indexes changed files in the \
    searched project(s) before answering and reports any project whose refresh \
    failed in `structuredContent.refresh` — hits still return, but a failed \
    project's results may lag disk. Each hit carries a rich, <mark>-highlighted \
    excerpt — enough to answer without a follow-up read, never a bare path list \
    or a whole file.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Full-text query" },
                "project": { "type": "string", "description": "Optional project id to narrow the search to" },
                "limit": { "type": "integer", "description": "Max hits to return (default 10)" }
            },
            "required": ["query"]
        }
    })
}

fn projects_schema() -> Value {
    json!({
        "name": "waggledance_projects",
        "description": "List every registered project: id, name, root path, indexed file \
    count, and when it was last seen.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    })
}

fn ask_state_schema() -> Value {
    json!({
        "name": "waggledance_ask_state",
        "description": "Ask waggledance for a project's parsed bee state (active feature, \
    phase, open/blocked cells, recent decisions, sessions, handoff, attention) \
    without reading any .bee file yourself. Omit `project` to get a rollup across \
    every registered project.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Optional project id to narrow to a single project's full snapshot" }
            }
        }
    })
}

fn dispatch_schema() -> Value {
    json!({
        "name": "waggledance_dispatch",
        "description": "Dispatch a task to an agent pane in a project that has opted into \
    orchestrator dispatch (D6 — refused, naming the remedy, when the project or the terminal \
    surface has not been switched on). Either spawn a fresh agent via an operator-configured \
    preset label, or target an already-running pane by id — exactly one of `preset`/`pane_id`, \
    never both, and never a raw command. Returns a `run_id` immediately; poll completion with \
    `waggledance_await`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Registered project id" },
                "preset": { "type": "string", "description": "Operator-configured agent preset label to spawn a fresh pane — mutually exclusive with pane_id" },
                "pane_id": { "type": "string", "description": "An already-running pane id to target — mutually exclusive with preset" },
                "task": { "type": "string", "description": "The task text sent to the agent" }
            },
            "required": ["project", "task"]
        }
    })
}

fn await_schema() -> Value {
    json!({
        "name": "waggledance_await",
        "description": "Block until a dispatched run completes, blocks on a human, or the \
    timeout elapses — whichever comes first, at most 60 seconds regardless of the \
    `timeout_seconds` requested (a longer request is silently clamped, never honored, never an \
    error). Returns `status` (working/done/blocked/timeout) and the transcript `delta` since \
    dispatch.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "The run_id returned by waggledance_dispatch" },
                "timeout_seconds": { "type": "integer", "description": "Max seconds to wait, clamped to 60 server-side (default 60)" }
            },
            "required": ["run_id"]
        }
    })
}

fn runs_schema() -> Value {
    json!({
        "name": "waggledance_runs",
        "description": "List dispatched runs and their current status (read-only, D8), \
    optionally filtered to one project.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": { "type": "string", "description": "Optional project id to narrow to a single project's runs" }
            }
        }
    })
}

fn handle_tool_call(
    id: Option<Value>,
    engine: &Engine,
    orchestration: &mut Option<Orchestration>,
    req: &Value,
) -> Value {
    let args = req
        .get("params")
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(json!({}));
    let name = req
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    match name {
        "waggledance_view_file" => handle_view_file(id, engine, &args),
        "waggledance_search" => handle_search(id, engine, &args),
        "waggledance_projects" => handle_projects(id, engine),
        "waggledance_ask_state" => handle_ask_state(id, engine, &args),
        "waggledance_dispatch" => handle_dispatch(id, engine, orchestration, &args),
        "waggledance_await" => handle_await(id, engine, orchestration, &args),
        "waggledance_runs" => handle_runs(id, engine, &args),
        _ => err(id, -32602, "unknown tool"),
    }
}

fn handle_view_file(id: Option<Value>, engine: &Engine, args: &Value) -> Value {
    let root = args
        .get("project_root")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let rel = args
        .get("relative_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if root.is_empty() || rel.is_empty() {
        return tool_error(id, "project_root and relative_path are required");
    }

    match engine.view_file(Path::new(root), rel) {
        Ok(vf) => {
            // Ensure a daemon is up so the URL is actually viewable. When the
            // daemon binds a wildcard host with no host_name override, this is
            // one URL per reachable machine IP so the caller can pick a routable
            // address; otherwise it is a single URL.
            let bases = runtime::ensure_daemon_bases();
            let urls: Vec<String> = bases
                .iter()
                .map(|base| format!("{base}/s/{}", vf.code))
                .collect();
            let long_urls: Vec<String> = bases
                .iter()
                .map(|base| format!("{base}{}", vf.url))
                .collect();
            // Primary URL kept for back-compat with clients reading `url`.
            let primary = urls.first().cloned().unwrap_or_default();
            let text = viewable_text(&urls, &vf.rel_path, &vf.project_id);
            ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "structuredContent": {
                        "url": primary,
                        "urls": urls,
                        "long_url": long_urls.first().cloned().unwrap_or_default(),
                        "long_urls": long_urls,
                        "path": vf.url,
                        "code": vf.code,
                        "project_id": vf.project_id
                    }
                }),
            )
        }
        Err(e) => tool_error(id, &format!("view_file failed: {e}")),
    }
}

/// `waggledance_search`: FTS5 hits over one or every registered project.
///
/// D4 (never silently stale): re-indexes the searched project(s) before
/// querying — just the filtered project, or every registered project when
/// unfiltered (D1) — and reports which projects refreshed cleanly and which
/// failed (review P1-2: a refresh failure must surface, never masquerade as
/// a fresh result). D2 (rich, not bare): each hit carries `project_id`,
/// `rel_path`, `title`, a `<mark>`-highlighted `excerpt`, and `score` — no
/// whole-file content, no bare path list.
fn handle_search(id: Option<Value>, engine: &Engine, args: &Value) -> Value {
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    if query.trim().is_empty() {
        return tool_error(id, "query is required");
    }
    let project = args
        .get("project")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_SEARCH_LIMIT);

    let refresh_results: Vec<(String, Result<usize, Error>)> = match project {
        Some(project_id) => {
            if engine.get_project(project_id).ok().flatten().is_none() {
                return tool_error(id, &format!("no such project: {project_id}"));
            }
            // The search itself still runs either way — a refresh failure is
            // reported, not fatal (results just stay as fresh as the last
            // successful index for that project).
            vec![(project_id.to_string(), engine.refresh_stale(project_id))]
        }
        None => {
            let Ok(projects) = engine.list_projects() else {
                return tool_error(id, "could not list registered projects");
            };
            projects
                .iter()
                .map(|p| (p.id.clone(), engine.refresh_stale(&p.id)))
                .collect()
        }
    };
    let refresh = summarize_refresh(refresh_results);

    match engine.search(query, project, limit) {
        Ok(hits) => {
            let mut text = if hits.is_empty() {
                format!("No hits for {query:?}.")
            } else {
                format!("{} hit(s) for {query:?}.", hits.len())
            };
            if let Some(warning) = &refresh.warning {
                text.push_str("; ");
                text.push_str(warning);
            }
            let structured_hits: Vec<Value> = hits
                .iter()
                .map(|h| {
                    json!({
                        "project_id": h.project_id,
                        "rel_path": h.rel_path,
                        "title": h.title,
                        "excerpt": h.excerpt,
                        "score": h.score
                    })
                })
                .collect();
            ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "structuredContent": { "hits": structured_hits, "refresh": refresh.structured }
                }),
            )
        }
        Err(e) => tool_error(id, &format!("search failed: {e}")),
    }
}

/// The per-project refresh outcome for a `waggledance_search` call, folded
/// into the shape the response carries and the one-line warning appended to
/// the human-readable text when at least one project's refresh failed
/// (review P1-2 — a stale-serving refresh failure must never be silent).
struct RefreshSummary {
    /// `structuredContent.refresh`: `{"refreshed": <ok count>, "failed": [...]}`.
    structured: Value,
    /// `Some` only when `failed` is non-empty.
    warning: Option<String>,
}

/// Pure so the failure shape is testable without a real store error: fold a
/// project id + its `refresh_stale` outcome into the response's `refresh`
/// field and, when any project failed, the warning appended to the text.
fn summarize_refresh(results: Vec<(String, Result<usize, Error>)>) -> RefreshSummary {
    let mut refreshed = 0usize;
    let mut failed: Vec<Value> = Vec::new();
    let mut failed_ids: Vec<String> = Vec::new();
    for (project_id, result) in results {
        match result {
            Ok(_) => refreshed += 1,
            Err(e) => {
                failed.push(json!({ "project_id": project_id, "error": e.to_string() }));
                failed_ids.push(project_id);
            }
        }
    }
    let warning = if failed_ids.is_empty() {
        None
    } else {
        Some(format!(
            "warning: refresh failed for {} — results may lag disk for those projects",
            failed_ids.join(", ")
        ))
    };
    RefreshSummary {
        structured: json!({ "refreshed": refreshed, "failed": failed }),
        warning,
    }
}

/// `waggledance_projects`: the registry, as-is. `file_count` reflects the
/// index as it stands and may lag until the next search touches a project
/// (recorded narrowing of D4 — plan.md Approach 3).
fn handle_projects(id: Option<Value>, engine: &Engine) -> Value {
    let projects = match engine.list_projects() {
        Ok(p) => p,
        Err(e) => return tool_error(id, &format!("could not list registered projects: {e}")),
    };
    let entries: Vec<Value> = projects
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "root_path": p.root_path.display().to_string(),
                "file_count": engine.file_count(&p.id).unwrap_or(0),
                "last_seen_at": p.last_seen_at
            })
        })
        .collect();
    let text = format!("{} registered project(s).", entries.len());
    ok(
        id,
        json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": { "projects": entries }
        }),
    )
}

/// `waggledance_ask_state`: parsed bee state, so the caller never opens a
/// `.bee/` file itself. With `project`: the full digest for that project
/// (`bee::read_snapshot`), including a project with no `.bee/` at all —
/// reported absent, never an error. Without `project`: a rollup across every
/// registered project (D1), via `bee::read_rollup`; `BeeProjectRollup` carries
/// no root/id of its own, so results are labeled by zipping the input roots'
/// projects back in by index (plan.md Approach 4).
fn handle_ask_state(id: Option<Value>, engine: &Engine, args: &Value) -> Value {
    let project = args
        .get("project")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    match project {
        Some(project_id) => {
            let Some(p) = engine.get_project(project_id).ok().flatten() else {
                return tool_error(id, &format!("no such project: {project_id}"));
            };
            let snapshot = bee::read_snapshot(&p.root_path);
            let digest = ask_state_digest(&p.id, &snapshot);
            let text = if !snapshot.present {
                format!("{}: no .bee/ directory (absent)", p.id)
            } else {
                let state = snapshot.state.as_ref();
                format!(
                    "{}: feature={:?} phase={:?} mode={:?} waiting_on_live={} \
                     doing={} waiting={} stuck={} done={}",
                    p.id,
                    state.and_then(|s| s.feature.as_deref()),
                    state.and_then(|s| s.phase.as_deref()),
                    state.and_then(|s| s.mode.as_deref()),
                    state.map(|s| s.waiting_on_live).unwrap_or(false),
                    snapshot.buckets.doing.len(),
                    snapshot.buckets.waiting.len(),
                    snapshot.buckets.stuck.len(),
                    snapshot.buckets.done.len(),
                )
            };
            ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "structuredContent": { "project": digest }
                }),
            )
        }
        None => {
            let Ok(projects) = engine.list_projects() else {
                return tool_error(id, "could not list registered projects");
            };
            let roots: Vec<std::path::PathBuf> =
                projects.iter().map(|p| p.root_path.clone()).collect();
            let rollups = bee::read_rollup(&roots);
            let digests: Vec<Value> = projects
                .iter()
                .zip(rollups.iter())
                .map(|(p, rollup)| ask_state_digest(&p.id, &rollup.snapshot))
                .collect();
            let text = format!("bee state rollup across {} project(s).", digests.len());
            ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "structuredContent": { "projects": digests }
                }),
            )
        }
    }
}

/// One project's `waggledance_ask_state` answer, built from a
/// [`bee::BeeSnapshot`]: feature/phase/mode, whether a human is currently
/// being waited on, cell bucket counts with doing/stuck detail, recent
/// decisions, sessions, handoff, and attention items. A project whose
/// `.bee/` is absent still gets this shape — every field just reads empty.
fn ask_state_digest(project_id: &str, snapshot: &bee::BeeSnapshot) -> Value {
    let state = snapshot.state.as_ref();
    let cell_line = |c: &bee::BeeCell| json!({ "id": c.id, "title": c.title });
    json!({
        "project_id": project_id,
        "present": snapshot.present,
        "feature": state.and_then(|s| s.feature.clone()),
        "phase": state.and_then(|s| s.phase.clone()),
        "mode": state.and_then(|s| s.mode.clone()),
        "waiting_on_live": state.map(|s| s.waiting_on_live).unwrap_or(false),
        "active": snapshot.active,
        "cell_counts": {
            "doing": snapshot.buckets.doing.len(),
            "waiting": snapshot.buckets.waiting.len(),
            "stuck": snapshot.buckets.stuck.len(),
            "done": snapshot.buckets.done.len()
        },
        "doing": snapshot.buckets.doing.iter().map(cell_line).collect::<Vec<_>>(),
        "stuck": snapshot.buckets.stuck.iter().map(cell_line).collect::<Vec<_>>(),
        "recent_decisions": snapshot.decisions.recent.iter().map(|d| json!({
            "id": d.id,
            "date": d.date,
            "decision": d.decision
        })).collect::<Vec<_>>(),
        "sessions": snapshot.sessions.iter().map(|s| json!({
            "id": s.id,
            "live": s.live,
            "heartbeat_age_minutes": s.heartbeat_age_minutes
        })).collect::<Vec<_>>(),
        "handoff": snapshot.handoff.as_ref().map(|h| json!({
            "kind": h.kind,
            "written_at": h.written_at,
            "next_action": h.next_action
        })),
        "attention": snapshot.attention.iter().map(|a| json!({
            "severity": format!("{:?}", a.severity),
            "title": a.title,
            "detail": a.detail
        })).collect::<Vec<_>>()
    })
}

/// The human-readable half of the tool result.
///
/// Pure on purpose: the caller resolves the daemon's base URLs (which starts a
/// daemon), so keeping the formatting separate is what makes this behaviour
/// testable at all.
///
/// The file's path rides along as ordinary text next to the short link, because
/// the link itself is opaque — without it, a transcript full of `/s/…` codes
/// tells a reader nothing about which document each one was.
/// `waggledance_dispatch`: D3/D6 gated dispatch. Refuses before touching
/// herdr at all when the project or the terminal family is off, or when the
/// preset label is unknown — only a resolved preset/pane_id ever reaches
/// [`run_dispatch`]'s herdr calls.
fn handle_dispatch(
    id: Option<Value>,
    engine: &Engine,
    orchestration: &mut Option<Orchestration>,
    args: &Value,
) -> Value {
    let Some(project_id) = args
        .get("project")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return tool_error(id, "\"project\" is required");
    };
    let Some(task) = args
        .get("task")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return tool_error(id, "\"task\" is required");
    };
    let preset_label = args
        .get("preset")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let pane_id_arg = args
        .get("pane_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    match (preset_label, pane_id_arg) {
        (Some(_), Some(_)) => {
            return tool_error(
                id,
                "specify exactly one of \"preset\" or \"pane_id\", not both",
            )
        }
        (None, None) => return tool_error(id, "specify one of \"preset\" or \"pane_id\""),
        _ => {}
    }

    let project = match engine.get_project(project_id) {
        Ok(Some(p)) => p,
        Ok(None) => return tool_error(id, &format!("project not found: {project_id}")),
        Err(e) => return tool_error(id, &format!("project lookup failed: {e}")),
    };
    if !engine.config.terminal.enabled {
        return tool_error(
            id,
            "the terminal surface is disabled — turn on terminal.enabled from the settings \
             page before dispatching",
        );
    }
    if !engine.orchestration_allowed(&project) {
        return tool_error(
            id,
            &format!(
                "project {project_id} has not opted into orchestrator dispatch — enable it \
                 from the project's settings page"
            ),
        );
    }

    // Resolved before any herdr call so an unknown label never touches the
    // socket at all.
    let preset = match preset_label {
        Some(label) => match engine
            .config
            .terminal
            .agent_presets
            .iter()
            .find(|p| p.label == label)
        {
            Some(p) => Some(p.clone()),
            None => return tool_error(id, &format!("unknown agent preset: {label}")),
        },
        None => None,
    };

    let orch = match orchestration_handle(orchestration, &engine.config.terminal) {
        Ok(o) => o,
        Err(e) => return tool_error(id, &e),
    };
    let outcome = orch.runtime.block_on(run_dispatch(
        &orch.herdr,
        engine,
        &project,
        preset,
        pane_id_arg.map(str::to_string),
        task,
    ));
    match outcome {
        Ok(run_id) => ok(
            id,
            json!({
                "content": [{ "type": "text", "text": format!("dispatched run {run_id}") }],
                "structuredContent": { "run_id": run_id }
            }),
        ),
        Err(msg) => tool_error(id, &msg),
    }
}

/// The dispatch protocol sequence itself (D1/D3/D5): resolve the target pane
/// (spawn via `preset`'s argv, or preflight an existing `pane_id`), capture
/// the baseline, mint a marker, send the task, and persist the run row (D7).
/// String-errored — every failure here is already a named, human-readable
/// refusal by the time it reaches [`handle_dispatch`].
async fn run_dispatch(
    herdr: &dyn Herdr,
    engine: &Engine,
    project: &Project,
    preset: Option<waggledance_core::config::AgentPreset>,
    pane_id_arg: Option<String>,
    task: &str,
) -> std::result::Result<String, String> {
    let (pane_id, preset_label) = match (preset, pane_id_arg) {
        (Some(preset), None) => {
            let snapshot = herdr
                .snapshot()
                .await
                .map_err(|e| format!("herdr snapshot failed: {e}"))?;
            let boundary =
                waggledance_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
                    .map_err(|e| format!("project {} destination unresolved: {e}", project.id))?;
            let (workspace_id, cwd) = orchestrate::resolve_spawn_destination(&snapshot, &boundary)
                .ok_or_else(|| {
                    format!(
                        "project {} has no herdr workspace with a resolved working directory \
                         under its own root; refusing to start an agent in an arbitrary \
                         directory",
                        project.id
                    )
                })?;
            let started = herdr
                .agent_start(&workspace_id, Some(&cwd), &preset.argv)
                .await
                .map_err(|e| format!("agent start failed: {e}"))?;
            (started.pane_id, Some(preset.label))
        }
        (None, Some(pane_id)) => {
            // D6 containment: a caller-supplied pane_id must belong to THIS
            // project's own root before any send. Without this, an opted-in
            // project could dispatch into any pane on the host (pane ids are
            // enumerable off GET /api/agents) -- the same boundary every
            // sibling pane-scoped write route enforces via
            // `project_and_verify_pane_in_boundary`. The spawn branch above
            // needs no equivalent: it creates the pane under a
            // boundary-validated workspace anchor.
            let snapshot = herdr
                .snapshot()
                .await
                .map_err(|e| format!("herdr snapshot failed: {e}"))?;
            let boundary =
                waggledance_core::paths_boundary::Boundary::new(vec![project.root_path.clone()])
                    .map_err(|e| format!("project {} destination unresolved: {e}", project.id))?;
            orchestrate::verify_pane_in_boundary(&snapshot, &boundary, &pane_id, &project.id)
                .map_err(|e| e.to_string())?;
            orchestrate::preflight(herdr, &pane_id)
                .await
                .map_err(|e| e.to_string())?;
            (pane_id, None)
        }
        (Some(_), Some(_)) | (None, None) => {
            unreachable!("handle_dispatch already validated exactly one of preset/pane_id")
        }
    };

    let baseline = orchestrate::capture_baseline(herdr, &pane_id)
        .await
        .map_err(|e| format!("herdr read failed: {e}"))?;
    let marker = orchestrate::mint_marker();
    orchestrate::send_task(herdr, &pane_id, task, &marker)
        .await
        .map_err(|e| format!("herdr send failed: {e}"))?;

    let now = waggledance_core::indexer::now_rfc3339();
    let run = Run {
        id: format!("run-{:016x}", rand::random::<u64>()),
        project_id: project.id.clone(),
        pane_id,
        preset_label,
        task: task.to_string(),
        baseline,
        marker: marker.joined(),
        status: "working".to_string(),
        created_at: now.clone(),
        updated_at: now,
    };
    let run_id = run.id.clone();
    engine
        .insert_run(&run)
        .map_err(|e| format!("run persistence failed: {e}"))?;
    Ok(run_id)
}

/// `waggledance_await`: bounded poll for a run's completion (D4/D5).
/// `run_id` is resolved before the orchestration runtime is ever built, so
/// an unknown id never touches herdr.
fn handle_await(
    id: Option<Value>,
    engine: &Engine,
    orchestration: &mut Option<Orchestration>,
    args: &Value,
) -> Value {
    let Some(run_id) = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return tool_error(id, "\"run_id\" is required");
    };
    let run = match engine.get_run(run_id) {
        Ok(Some(r)) => r,
        Ok(None) => return tool_error(id, &format!("unknown run_id: {run_id}")),
        Err(e) => return tool_error(id, &format!("run lookup failed: {e}")),
    };
    let timeout = args
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .map(std::time::Duration::from_secs)
        .unwrap_or(orchestrate::MAX_AWAIT_TIMEOUT);

    let orch = match orchestration_handle(orchestration, &engine.config.terminal) {
        Ok(o) => o,
        Err(e) => return tool_error(id, &e),
    };
    match orch.runtime.block_on(orchestrate::await_run(
        &orch.herdr,
        engine,
        &run,
        timeout,
        orch.notify_store.as_ref(),
    )) {
        Ok(outcome) => ok(
            id,
            json!({
                "content": [{ "type": "text", "text": format!("run {run_id}: {}", outcome.status.as_str()) }],
                "structuredContent": {
                    "run_id": run_id,
                    "status": outcome.status.as_str(),
                    "delta": outcome.delta
                }
            }),
        ),
        Err(e) => tool_error(id, &format!("await failed: {e}")),
    }
}

/// `waggledance_runs`: the run store, read-only (D8), optionally narrowed to
/// one project.
fn handle_runs(id: Option<Value>, engine: &Engine, args: &Value) -> Value {
    let project_filter = args
        .get("project")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let project_ids: Vec<String> = match project_filter {
        Some(p) => vec![p.to_string()],
        None => match engine.list_projects() {
            Ok(ps) => ps.into_iter().map(|p| p.id).collect(),
            Err(e) => return tool_error(id, &format!("could not list registered projects: {e}")),
        },
    };
    let mut runs: Vec<Run> = Vec::new();
    for pid in &project_ids {
        match engine.list_runs(pid, RUNS_LIST_LIMIT) {
            Ok(rs) => runs.extend(rs),
            Err(e) => return tool_error(id, &format!("run listing failed: {e}")),
        }
    }
    let rows: Vec<Value> = runs
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "project_id": r.project_id,
                "pane_id": r.pane_id,
                "preset_label": r.preset_label,
                "task": r.task,
                "status": r.status,
                "created_at": r.created_at,
                "updated_at": r.updated_at
            })
        })
        .collect();
    ok(
        id,
        json!({
            "content": [{ "type": "text", "text": format!("{} run(s)", rows.len()) }],
            "structuredContent": { "runs": rows }
        }),
    )
}

fn viewable_text(urls: &[String], rel_path: &str, project_id: &str) -> String {
    let viewable = if urls.len() > 1 {
        let lines = urls
            .iter()
            .map(|u| format!("  {rel_path} → {u}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Viewable at (pick a reachable IP):\n{lines}")
    } else {
        let primary = urls.first().map(String::as_str).unwrap_or_default();
        format!("Viewable at: {rel_path} → {primary}")
    };
    format!("{viewable}\nproject_id: {project_id}")
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}
fn err(id: Option<Value>, code: i64, msg: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } })
}
/// Tool-level error: reported inside a successful result with isError=true (MCP convention).
fn tool_error(id: Option<Value>, msg: &str) -> Value {
    ok(
        id,
        json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_base_renders_a_single_line() {
        let text = viewable_text(
            &["http://design-lap:7700/s/a3f9c1d20b74".into()],
            "docs/history/short-link/DISCUSSION.md",
            "waggledance",
        );
        assert_eq!(
            text,
            "Viewable at: docs/history/short-link/DISCUSSION.md → \
             http://design-lap:7700/s/a3f9c1d20b74\nproject_id: waggledance"
        );
        assert_eq!(text.lines().count(), 2);
    }

    #[test]
    fn several_bases_render_one_line_each() {
        let text = viewable_text(
            &[
                "http://192.168.1.10:7700/s/a3f9c1d20b74".into(),
                "http://10.0.0.5:7700/s/a3f9c1d20b74".into(),
            ],
            "docs/a.md",
            "waggledance",
        );
        assert!(text.contains("pick a reachable IP"));
        assert!(text.contains("  docs/a.md → http://192.168.1.10:7700/s/a3f9c1d20b74"));
        assert!(text.contains("  docs/a.md → http://10.0.0.5:7700/s/a3f9c1d20b74"));
    }

    /// The whole point of the feature: the emitted line has to stay inside a
    /// terminal width, which the full path did not.
    #[test]
    fn the_short_line_fits_in_a_terminal() {
        let deep = "docs/history/short-link-for-file-urls/DISCUSSION.md";
        let text = viewable_text(
            &["http://design-lap:7700/s/a3f9c1d20b74".into()],
            deep,
            "waggledance",
        );
        let url_line = text.lines().next().unwrap();
        let url = url_line.split(" → ").nth(1).unwrap();
        assert!(url.len() <= 40, "short url grew to {}: {url}", url.len());
    }

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn call_tool(engine: &Engine, name: &str, args: Value) -> Value {
        call_tool_with_orchestration(engine, &mut None, name, args)
    }

    fn call_tool_with_orchestration(
        engine: &Engine,
        orchestration: &mut Option<Orchestration>,
        name: &str,
        args: Value,
    ) -> Value {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });
        handle_tool_call(Some(json!(1)), engine, orchestration, &req)
    }

    /// Two registered projects, each with one markdown file sharing a word
    /// ("grapefruit") that appears nowhere else — a search unfiltered must
    /// span both (D1); a filtered search must narrow to one.
    fn two_project_engine(
        tag: &str,
    ) -> (
        Engine,
        waggledance_core::domain::Project,
        waggledance_core::domain::Project,
    ) {
        let dir_a =
            std::env::temp_dir().join(format!("waggledance-mcp-{tag}-a-{}", std::process::id()));
        let dir_b =
            std::env::temp_dir().join(format!("waggledance-mcp-{tag}-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
        write(
            &dir_a,
            "docs/a.md",
            "# Project A\nThe grapefruit orchard thrives in spring.",
        );
        write(
            &dir_b,
            "docs/b.md",
            "# Project B\nA grapefruit smoothie recipe for summer.",
        );

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let pa = engine.register(&dir_a, None).unwrap();
        let pb = engine.register(&dir_b, None).unwrap();
        (engine, pa, pb)
    }

    #[test]
    fn tools_list_has_seven_schemas() {
        let tools = [
            view_file_schema(),
            search_schema(),
            projects_schema(),
            ask_state_schema(),
            dispatch_schema(),
            await_schema(),
            runs_schema(),
        ];
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "waggledance_view_file",
                "waggledance_search",
                "waggledance_projects",
                "waggledance_ask_state",
                "waggledance_dispatch",
                "waggledance_await",
                "waggledance_runs"
            ]
        );
    }

    #[test]
    fn search_unfiltered_spans_multiple_projects_with_marked_excerpts() {
        let (engine, pa, pb) = two_project_engine("search-multi");
        let resp = call_tool(
            &engine,
            "waggledance_search",
            json!({ "query": "grapefruit" }),
        );
        let hits = resp["result"]["structuredContent"]["hits"]
            .as_array()
            .unwrap();
        assert_eq!(hits.len(), 2, "expected a hit from each project: {resp}");
        let ids: Vec<&str> = hits
            .iter()
            .map(|h| h["project_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&pa.id.as_str()));
        assert!(ids.contains(&pb.id.as_str()));
        for h in hits {
            assert!(h["excerpt"].as_str().unwrap().contains("<mark>"));
        }

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn search_project_filter_narrows_to_one() {
        let (engine, pa, pb) = two_project_engine("search-filter");
        let resp = call_tool(
            &engine,
            "waggledance_search",
            json!({ "query": "grapefruit", "project": pa.id }),
        );
        let hits = resp["result"]["structuredContent"]["hits"]
            .as_array()
            .unwrap();
        assert_eq!(hits.len(), 1, "expected only project a's hit: {resp}");
        assert_eq!(hits[0]["project_id"], pa.id);

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn search_reflects_a_file_edited_on_disk_since_last_index() {
        let (engine, pa, pb) = two_project_engine("search-stale");
        // Edit project a's file after registration (which already indexed
        // it once) — D4: the next search must see the new content without a
        // separate refresh call.
        write(
            &pa.root_path,
            "docs/a.md",
            "# Project A\nNow mentions pineapple instead.",
        );
        let resp = call_tool(
            &engine,
            "waggledance_search",
            json!({ "query": "pineapple" }),
        );
        let hits = resp["result"]["structuredContent"]["hits"]
            .as_array()
            .unwrap();
        assert_eq!(hits.len(), 1, "edited content must be searchable: {resp}");
        assert_eq!(hits[0]["project_id"], pa.id);

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn fts_hostile_query_returns_empty_not_error() {
        let (engine, pa, pb) = two_project_engine("search-hostile");
        let resp = call_tool(&engine, "waggledance_search", json!({ "query": "*)(" }));
        assert!(
            resp["result"]["isError"].is_null(),
            "unexpected error: {resp}"
        );
        let hits = resp["result"]["structuredContent"]["hits"]
            .as_array()
            .unwrap();
        assert!(
            hits.is_empty(),
            "hostile query must not error, and must not match: {resp}"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn search_missing_query_is_a_tool_error() {
        let (engine, pa, pb) = two_project_engine("search-missing-query");
        let resp = call_tool(&engine, "waggledance_search", json!({}));
        assert_eq!(resp["result"]["isError"], true, "{resp}");

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn search_nonexistent_project_is_a_tool_error_naming_it() {
        let (engine, pa, pb) = two_project_engine("search-no-project");
        let resp = call_tool(
            &engine,
            "waggledance_search",
            json!({ "query": "grapefruit", "project": "does-not-exist" }),
        );
        assert_eq!(resp["result"]["isError"], true, "{resp}");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("does-not-exist"), "{text}");

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    /// Happy path (review P1-2): a clean unfiltered search reports both
    /// registered projects refreshed and names none as failed.
    #[test]
    fn search_reports_a_clean_refresh_outcome() {
        let (engine, pa, pb) = two_project_engine("search-refresh-happy");
        let resp = call_tool(
            &engine,
            "waggledance_search",
            json!({ "query": "grapefruit" }),
        );
        let refresh = &resp["result"]["structuredContent"]["refresh"];
        assert!(
            refresh["refreshed"].as_u64().unwrap() >= 1,
            "expected at least one project refreshed: {resp}"
        );
        assert_eq!(refresh["failed"].as_array().unwrap().len(), 0, "{resp}");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("warning"), "{text}");

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    /// Failure path (review P1-2): `summarize_refresh` is the seam a real
    /// `refresh_stale` error (DB locked past busy_timeout, store error
    /// mid-walk) folds through — asserted directly since inducing a real
    /// store failure from this test would be unreliable.
    #[test]
    fn summarize_refresh_surfaces_a_failed_project_and_a_warning() {
        let summary = summarize_refresh(vec![
            ("ok-project".to_string(), Ok(3)),
            (
                "broken-project".to_string(),
                Err(Error::Other("db locked".to_string())),
            ),
        ]);
        assert_eq!(summary.structured["refreshed"], 1);
        let failed = summary.structured["failed"].as_array().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["project_id"], "broken-project");
        assert_eq!(failed[0]["error"], "db locked");
        let warning = summary.warning.expect("a failed project must warn");
        assert!(warning.contains("broken-project"), "{warning}");
        assert!(warning.contains("may lag disk"), "{warning}");
    }

    /// A search response with no failed projects carries no warning.
    #[test]
    fn summarize_refresh_is_silent_when_nothing_failed() {
        let summary = summarize_refresh(vec![("a".to_string(), Ok(1)), ("b".to_string(), Ok(0))]);
        assert_eq!(summary.structured["refreshed"], 2);
        assert_eq!(summary.structured["failed"].as_array().unwrap().len(), 0);
        assert!(summary.warning.is_none());
    }

    #[test]
    fn projects_lists_both_with_counts_and_root() {
        let (engine, pa, pb) = two_project_engine("projects-list");
        let resp = call_tool(&engine, "waggledance_projects", json!({}));
        let entries = resp["result"]["structuredContent"]["projects"]
            .as_array()
            .unwrap();
        assert_eq!(entries.len(), 2, "{resp}");
        let a = entries.iter().find(|e| e["id"] == pa.id).unwrap();
        assert_eq!(a["file_count"], 1);
        assert_eq!(a["root_path"], pa.root_path.display().to_string());
        assert!(a["last_seen_at"].as_str().is_some());

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn ask_state_filtered_reads_feature_phase_and_buckets_without_a_direct_read() {
        let (engine, pa, pb) = two_project_engine("ask-state-filtered");
        write(
            &pa.root_path,
            ".bee/state.json",
            r#"{"feature": "widget-polish", "phase": "execution", "mode": "standard"}"#,
        );
        let resp = call_tool(
            &engine,
            "waggledance_ask_state",
            json!({ "project": pa.id }),
        );
        let digest = &resp["result"]["structuredContent"]["project"];
        assert_eq!(digest["present"], true, "{resp}");
        assert_eq!(digest["feature"], "widget-polish");
        assert_eq!(digest["phase"], "execution");
        assert_eq!(digest["mode"], "standard");
        assert_eq!(digest["cell_counts"]["doing"], 0);

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn ask_state_unfiltered_rolls_up_every_project_including_one_with_no_bee_dir() {
        let (engine, pa, pb) = two_project_engine("ask-state-rollup");
        write(
            &pa.root_path,
            ".bee/state.json",
            r#"{"feature": "widget-polish", "phase": "execution", "mode": "standard"}"#,
        );
        // project b deliberately has no .bee/ at all.
        let resp = call_tool(&engine, "waggledance_ask_state", json!({}));
        let entries = resp["result"]["structuredContent"]["projects"]
            .as_array()
            .unwrap();
        assert_eq!(entries.len(), 2, "{resp}");
        let a = entries.iter().find(|e| e["project_id"] == pa.id).unwrap();
        assert_eq!(a["present"], true);
        assert_eq!(a["feature"], "widget-polish");
        let b = entries.iter().find(|e| e["project_id"] == pb.id).unwrap();
        assert_eq!(
            b["present"], false,
            "absent .bee/ must report absent, not error: {b}"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn ask_state_nonexistent_project_is_a_tool_error_naming_it() {
        let (engine, pa, pb) = two_project_engine("ask-state-no-project");
        let resp = call_tool(
            &engine,
            "waggledance_ask_state",
            json!({ "project": "does-not-exist" }),
        );
        assert_eq!(resp["result"]["isError"], true, "{resp}");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("does-not-exist"), "{text}");

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn unknown_tool_stays_on_the_json_rpc_error_path() {
        let (engine, pa, pb) = two_project_engine("unknown-tool");
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "not_a_real_tool", "arguments": {} }
        });
        let resp = handle_tool_call(Some(json!(1)), &engine, &mut None, &req);
        assert_eq!(resp["error"]["code"], -32602, "{resp}");
        assert!(
            resp["result"].is_null(),
            "unknown tool must not be a tool_error: {resp}"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    /// A single-project engine with the terminal family already on
    /// (`Config::default()` leaves it off) — the dispatch-family refusal
    /// tests below build on this and flip the per-project D6 switch
    /// themselves where a test needs it on too.
    fn dispatch_engine(tag: &str) -> (Engine, waggledance_core::domain::Project) {
        let dir =
            std::env::temp_dir().join(format!("waggledance-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# Project\nnothing interesting.");
        let mut config = Config::default();
        config.terminal.enabled = true;
        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), config);
        let project = engine.register(&dir, None).unwrap();
        (engine, project)
    }

    #[test]
    fn dispatch_requires_project_and_task() {
        let (engine, pa) = dispatch_engine("dispatch-required-fields");
        let missing_project = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "task": "do the thing", "pane_id": "w1:p1" }),
        );
        assert_eq!(
            missing_project["result"]["isError"], true,
            "{missing_project}"
        );
        let missing_task = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "project": pa.id, "pane_id": "w1:p1" }),
        );
        assert_eq!(missing_task["result"]["isError"], true, "{missing_task}");

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    #[test]
    fn dispatch_requires_exactly_one_of_preset_or_pane_id() {
        let (engine, pa) = dispatch_engine("dispatch-preset-xor-pane");
        let neither = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "project": pa.id, "task": "go" }),
        );
        let neither_text = neither["result"]["content"][0]["text"].as_str().unwrap();
        assert!(neither_text.contains("specify one of"), "{neither_text}");

        let both = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "project": pa.id, "task": "go", "preset": "p", "pane_id": "w1:p1" }),
        );
        let both_text = both["result"]["content"][0]["text"].as_str().unwrap();
        assert!(both_text.contains("not both"), "{both_text}");

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    #[test]
    fn dispatch_refuses_an_unknown_project() {
        let (engine, pa) = dispatch_engine("dispatch-unknown-project");
        let resp = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "project": "does-not-exist", "task": "go", "pane_id": "w1:p1" }),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("does-not-exist"), "{text}");

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    /// D6: the terminal family is off by default even on an
    /// orchestration-enabled project — the refusal must name the remedy.
    #[test]
    fn dispatch_refuses_when_terminal_family_is_off() {
        let dir = std::env::temp_dir().join(format!(
            "waggledance-mcp-dispatch-terminal-off-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# Project\n");
        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        engine.set_orchestration_enabled(&project.id, true).unwrap();

        let resp = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "project": project.id, "task": "go", "pane_id": "w1:p1" }),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("terminal.enabled"), "{text}");

        std::fs::remove_dir_all(&project.root_path).ok();
    }

    /// D6: terminal family on, but this project never opted in — the
    /// refusal must name the remedy (the project's own settings page).
    #[test]
    fn dispatch_refuses_a_project_that_has_not_opted_in() {
        let (engine, pa) = dispatch_engine("dispatch-not-opted-in");
        let resp = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "project": pa.id, "task": "go", "pane_id": "w1:p1" }),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("has not opted into orchestrator dispatch"),
            "{text}"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    #[test]
    fn dispatch_refuses_an_unknown_preset_label_before_touching_herdr() {
        let (engine, pa) = dispatch_engine("dispatch-unknown-preset");
        engine.set_orchestration_enabled(&pa.id, true).unwrap();

        let resp = call_tool(
            &engine,
            "waggledance_dispatch",
            json!({ "project": pa.id, "task": "go", "preset": "nope" }),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("unknown agent preset: nope"), "{text}");

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    /// Targeting a pane_id goes through `orchestrate::preflight` before any
    /// send — this proves that wiring end to end against whatever herdr
    /// state the test sandbox actually has: no socket at all reports
    /// unverifiable/unavailable, a live socket with no such pane reports
    /// `no such pane` — either way a named error, never a silent hang and
    /// never a fabricated completion (D5 fail-closed).
    #[test]
    fn dispatch_to_an_unreachable_pane_refuses_fail_closed_never_a_completion() {
        let (engine, pa) = dispatch_engine("dispatch-herdr-unavailable");
        engine.set_orchestration_enabled(&pa.id, true).unwrap();

        let mut orchestration: Option<Orchestration> = None;
        let resp = call_tool_with_orchestration(
            &engine,
            &mut orchestration,
            "waggledance_dispatch",
            json!({ "project": pa.id, "task": "go", "pane_id": "w1:p1" }),
        );
        assert_eq!(resp["result"]["isError"], true, "{resp}");
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let lower = text.to_lowercase();
        assert!(
            lower.contains("unverifiable")
                || lower.contains("herdr")
                || lower.contains("no such pane"),
            "expected a named fail-closed refusal, got: {text}"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    #[test]
    fn await_requires_run_id() {
        let (engine, pa) = dispatch_engine("await-required-field");
        let resp = call_tool(&engine, "waggledance_await", json!({}));
        assert_eq!(resp["result"]["isError"], true, "{resp}");

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    /// An unknown run_id never touches herdr — resolved before the
    /// orchestration runtime is built.
    #[test]
    fn await_refuses_an_unknown_run_id_without_touching_herdr() {
        let (engine, pa) = dispatch_engine("await-unknown-run");
        let resp = call_tool(
            &engine,
            "waggledance_await",
            json!({ "run_id": "does-not-exist" }),
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("unknown run_id: does-not-exist"), "{text}");

        std::fs::remove_dir_all(&pa.root_path).ok();
    }

    fn seeded_run(project_id: &str, id: &str) -> Run {
        let now = waggledance_core::indexer::now_rfc3339();
        Run {
            id: id.to_string(),
            project_id: project_id.to_string(),
            pane_id: "w1:p1".to_string(),
            preset_label: None,
            task: "do the thing".to_string(),
            baseline: String::new(),
            marker: "HERDR_DONE_deadbeef".to_string(),
            status: "working".to_string(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn runs_filtered_lists_only_that_projects_rows() {
        let (engine, pa, pb) = two_project_engine("runs-filtered");
        engine.insert_run(&seeded_run(&pa.id, "run-a")).unwrap();
        engine.insert_run(&seeded_run(&pb.id, "run-b")).unwrap();

        let resp = call_tool(&engine, "waggledance_runs", json!({ "project": pa.id }));
        let rows = resp["result"]["structuredContent"]["runs"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1, "{resp}");
        assert_eq!(rows[0]["id"], "run-a");

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    #[test]
    fn runs_unfiltered_spans_every_project() {
        let (engine, pa, pb) = two_project_engine("runs-unfiltered");
        engine.insert_run(&seeded_run(&pa.id, "run-a")).unwrap();
        engine.insert_run(&seeded_run(&pb.id, "run-b")).unwrap();

        let resp = call_tool(&engine, "waggledance_runs", json!({}));
        let rows = resp["result"]["structuredContent"]["runs"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 2, "{resp}");

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    /// D6 / dbn-4: in MCP stdio process, Orchestration opens its own NotifyStore
    /// against the server's database when notifications are enabled, and None
    /// when disabled so the await path raises no alerts.
    #[test]
    fn await_path_receives_store_when_notify_enabled_and_none_when_disabled() {
        // 1. Switch off: orchestration initialized on await path has no notify store
        let (engine_off, pa) = dispatch_engine("await-notify-off");
        assert!(!engine_off.config.terminal.notify_enabled);
        let mut orch_off: Option<Orchestration> = None;
        let orch = orchestration_handle(&mut orch_off, &engine_off.config.terminal).unwrap();
        assert!(
            orch.notify_store.is_none(),
            "D6: notify switch off must not open a notify store on the await path"
        );

        // 2. Switch on: orchestration initialized on await path receives a live notify store
        let mut config_on = Config::default();
        config_on.terminal.enabled = true;
        config_on.terminal.notify_enabled = true;
        let dir_on = std::env::temp_dir().join(format!(
            "waggledance-mcp-await-notify-on-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir_on);
        write(&dir_on, "docs/a.md", "# Project\n");
        let engine_on = Engine::new(SqliteStore::open_in_memory().unwrap(), config_on);
        let pb = engine_on.register(&dir_on, None).unwrap();
        let mut orch_on: Option<Orchestration> = None;
        let orch = orchestration_handle(&mut orch_on, &engine_on.config.terminal).unwrap();
        assert!(
            orch.notify_store.is_some(),
            "notify switch on must open a notify store for the await path"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }

    /// D6 / dbn-4: `open_notify_store` opens lazily only when `notify_enabled` is true,
    /// and leaves no database file when it is false.
    #[test]
    fn notify_store_in_mcp_opens_only_when_notify_switch_is_on() {
        let dir = std::env::temp_dir().join(format!(
            "waggledance-mcp-notify-store-lazy-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = waggledance_core::config::notify_store_path_override(Some(&dir));

        let off = waggledance_core::config::TerminalConfig::default();
        assert!(!off.notify_enabled);
        let store_off = open_notify_store(&off, Some(&dir));
        assert!(store_off.is_none(), "switch off must return None");
        assert!(
            !path.exists(),
            "opening store with notify switch off must not create database file"
        );

        let on = waggledance_core::config::TerminalConfig {
            notify_enabled: true,
            ..Default::default()
        };
        let store_on = open_notify_store(&on, Some(&dir));
        assert!(store_on.is_some(), "switch on must return Some(store)");
        assert!(
            path.exists(),
            "opening store with notify switch on must create database file"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// D6 / dbn-4: calling `waggledance_await` with notify switch on arms Orchestration
    /// with a notify store, and with it off leaves notify_store None.
    #[test]
    fn await_tool_call_arms_notify_store_under_opt_in_switch() {
        let (engine_off, pa) = dispatch_engine("await-tool-off");
        engine_off
            .insert_run(&seeded_run(&pa.id, "run-tool-off"))
            .unwrap();
        let mut orch_off: Option<Orchestration> = None;
        let _ = call_tool_with_orchestration(
            &engine_off,
            &mut orch_off,
            "waggledance_await",
            json!({ "run_id": "run-tool-off", "timeout_seconds": 0 }),
        );
        let orch = orch_off
            .as_ref()
            .expect("orchestration initialized by await");
        assert!(
            orch.notify_store.is_none(),
            "disabled notify switch -> no store"
        );

        let dir = std::env::temp_dir().join(format!(
            "waggledance-mcp-await-tool-on-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# Project\n");
        let mut config_on = Config::default();
        config_on.terminal.enabled = true;
        config_on.terminal.notify_enabled = true;
        let engine_on = Engine::new(SqliteStore::open_in_memory().unwrap(), config_on);
        let pb = engine_on.register(&dir, None).unwrap();
        engine_on
            .insert_run(&seeded_run(&pb.id, "run-tool-on"))
            .unwrap();

        let mut orch_on: Option<Orchestration> = None;
        let _ = call_tool_with_orchestration(
            &engine_on,
            &mut orch_on,
            "waggledance_await",
            json!({ "run_id": "run-tool-on", "timeout_seconds": 0 }),
        );
        let orch = orch_on
            .as_ref()
            .expect("orchestration initialized by await");
        assert!(
            orch.notify_store.is_some(),
            "enabled notify switch -> Some(store)"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }
}
