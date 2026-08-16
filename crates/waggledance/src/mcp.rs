//! Minimal MCP server over stdio (newline-delimited JSON-RPC 2.0).
//! Exposes the agent-facing query surface (PRD §5.5): the original write-side
//! `waggledance_view_file`, plus three read-only query tools —
//! `waggledance_search`, `waggledance_projects`, `waggledance_ask_state`
//! (mcp-query-surface D3). Hand-rolled to avoid a heavy SDK dependency; the
//! protocol surface here is intentionally small.

use crate::runtime;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::Path;
use waggledance_core::bee;
use waggledance_core::config::registry_db_path;
use waggledance_core::{Config, Engine, Error, SqliteStore};

/// Default `waggledance_search` hit cap when the caller does not pass `limit`.
const DEFAULT_SEARCH_LIMIT: usize = 10;

const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn run() -> Result<()> {
    let engine = Engine::new(SqliteStore::open(&registry_db_path())?, Config::load());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

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
                        ask_state_schema()
                    ]
                }),
            )),
            "tools/call" => Some(handle_tool_call(id, &engine, &req)),
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

fn handle_tool_call(id: Option<Value>, engine: &Engine, req: &Value) -> Value {
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
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });
        handle_tool_call(Some(json!(1)), engine, &req)
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
    fn tools_list_has_four_schemas() {
        let tools = [
            view_file_schema(),
            search_schema(),
            projects_schema(),
            ask_state_schema(),
        ];
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "waggledance_view_file",
                "waggledance_search",
                "waggledance_projects",
                "waggledance_ask_state"
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
        let resp = handle_tool_call(Some(json!(1)), &engine, &req);
        assert_eq!(resp["error"]["code"], -32602, "{resp}");
        assert!(
            resp["result"].is_null(),
            "unknown tool must not be a tool_error: {resp}"
        );

        std::fs::remove_dir_all(&pa.root_path).ok();
        std::fs::remove_dir_all(&pb.root_path).ok();
    }
}
