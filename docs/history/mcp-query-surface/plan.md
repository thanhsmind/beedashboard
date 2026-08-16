---
artifact_contract: bee-plan/v1
mode: standard
---

# Plan: MCP Query Surface

Mode: `standard` — 1 risk flag: public-contracts (new agent-facing MCP tool contract)
Why this is the least workflow that protects the work: a new external contract across 4 product files deserves a written shape and a review wave, but no hard-gate territory is touched.

## Requirements (from CONTEXT.md)

- D1: Query tools default to ALL registered projects; optional project filter narrows.
- D2: Search hits return path anchor + rich snippet sufficient to answer without a follow-up Read; never bare path lists, never whole files by default.
- D3: v1 ships three tools together — `waggledance_search`, `waggledance_projects`, `waggledance_ask_state`.
- D4: A query re-indexes changed files in the touched project(s) before returning; never stale results.

## Discovery

Gather digest (bee-gather, 2026-08-16) over mcp.rs / repository.rs / indexer.rs / engine.rs / bee.rs / server.rs:

- MCP server is stdio JSON-RPC with a single-tool schema and a `name` match in `handle_tool_call` (`crates/waggledance/src/mcp.rs:58,75`); new tools extend `tools/list` and branch there.
- `Engine::search` → `SqliteStore::search` already does FTS5 + bm25 + `snippet(...)` — but with a 12-token window (`crates/waggledance-core/src/repository.rs:305-312`), too small for D2, and **no path re-indexes before searching** (D4 gap; `engine.rs:249`, `server.rs:4271`, `cli.rs:386`).
- `IndexedFile` stores `size_bytes` + `modified_at` (`domain.rs:19-29`), enabling an mtime/size-based stale check; today `index_project` re-reads every file unconditionally (`indexer.rs:21-35`).
- bee state is already modeled: `read_snapshot(root)` per project, `read_rollup(roots)` cross-project (`bee.rs:1023,1269`) with feature/phase, cell buckets, decisions, sessions, waiting_on_live.
- `cli.rs:386` `cmd_search` is the leanest call shape to mirror; `server.rs:1069` `api_projects` is the projects-listing shape (MCP is local/trusted, so `root_path` may be included).

## Approach

Recommended (cites D1–D4): expose the existing engines through three new MCP tools, adding only the two missing capabilities — a bounded stale-refresh (D4) and a richer snippet (D2).

1. `Engine::refresh_stale(project_id)` — walk the project like `index_project`, but re-read/upsert only files whose fs mtime or size differs from the stored `files` row; new files indexed, deleted files removed. Constraints from the review wave: (a) the mtime compare goes through the exact same `OffsetDateTime::from(t).format(&Rfc3339)` path the indexer stores (`indexer.rs:58-62`) — a naive compare marks everything changed; (b) skip the delete pass when the walk yields 0 files against a non-empty index or the `root_path` doesn't exist — a vanished/unmounted root must never empty its project's index; (c) files `index_file` declines to store (oversize / unreadable / non-UTF8, `indexer.rs:43-52`) must not be content-read on every query — the size check already precedes the read; unreadable/non-UTF8 get a skip rule (writer's choice of marker). `waggledance_search` calls it for the filtered project, or every registered project when unfiltered (D1+D4). Measured basis (review wave): 10 projects / 4920 indexed files, plain `find` over all roots 0.62s warm — budget: the unfiltered stale-walk stays under ~2s warm-cache; record the actual number in the cell outcome.
2. Snippet window: raise the FTS5 `snippet()` window from 12 to 64 tokens (the documented FTS5 ceiling, verified against bundled sqlite 3.45.3) in `SqliteStore::search` (single shared path; web `_search` inherits the richer excerpt — escaping is safe per `views.rs:5944-5948`; the search card gets ~5x longer text, accepted as-is, no clamp). Hit shape (D2): `project_id`, `rel_path`, `title`, `excerpt` (with `<mark>` markers), `score`.
3. `waggledance_projects` — `list_projects()` + `file_count(id)` + `root_path` + `last_seen_at`. Recorded narrowing of D4: the freshness guarantee attaches to `waggledance_search` results; `file_count` here reflects the index as-is and may lag until the next search touches that project.
4. `waggledance_ask_state` — no project arg: `read_rollup` over all registered roots (D1); `BeeProjectRollup` carries no root/id (`bee.rs:1247-1250`), so results are labeled by zipping the input roots by index. With project arg: full `read_snapshot` digest (feature, phase, mode, waiting_on_live, cell buckets with doing/stuck detail, recent decisions, sessions, handoff, attention). Acknowledged per D1: the unfiltered default hands parsed bee state of every registered project (including unrelated repos) to any local MCP caller — same local-trust stance as exposing `root_path`. `read_rollup` is synchronous; N roots read serially on the MCP thread, stacking onto refresh latency — accepted for v1.
5. All three follow the existing result convention: `content` text + `structuredContent` (`mcp.rs:119-133`). Two distinct error shapes, not one: unknown tool keeps the JSON-RPC `err()` `-32602` path (`mcp.rs:87,166-168`); bad/missing arguments and nonexistent-project use `tool_error()` (`isError: true`, `mcp.rs:170-175`).
6. mcp.rs prerequisite refactor (Phase 2, first step): the current handler is an early-return guard with the `view_file` body inlined (`mcp.rs:86-134`) — extract the body into its own fn, convert the guard to a `match name`, and turn `tools/list`'s hardcoded single-schema array (`mcp.rs:43`) into a four-schema vec.
7. Cross-process safety: the MCP process and the daemon both open `registry.db`; WAL is on but no `busy_timeout` is set anywhere (grep verified), so routine writes from unfiltered searches would collide with the watcher's reindex as immediate SQLITE_BUSY. Phase 1 sets a `busy_timeout` pragma on connection open in `SqliteStore`.

Rejected alternatives:
- Full `Engine::refresh` before every search — honors D4 but re-reads every file of every project per query; unacceptable latency at all-projects default.
- Separate `search_rich` store method to protect the web UI's 12-token snippet — two code paths for one behavior; the web UI benefiting from richer snippets is not a regression.
- HTTP API instead of MCP tools — agents already speak MCP to this server; a second transport adds surface without adding capability.

Risk map: repository.rs snippet change / LOW / existing FTS test + one asserting window; refresh_stale / MEDIUM (fs-time semantics) / unit tests over mtime+size change matrix; mcp.rs dispatch / LOW / dispatch unit tests; bee.rs reuse / LOW (read-only, already dashboard-proven).

## Shape

Phase plan (milestone-shaped; single slice — walking skeleton is the whole v1):

| Phase | What Changes | Why Now | Demo | Unlocks |
|---|---|---|---|---|
| 1. Core capabilities | `Engine::refresh_stale` (mtime/size selective re-index); snippet window 12→64; any small accessors the tools need | Both are the only missing engine pieces; everything else exists | `cargo test`: stale-file matrix green; snippet test shows ≥12-token context | Phase 2 handlers stay thin |
| 2. MCP tools | `tools/list` returns 4 schemas; `handle_tool_call` branches to search (refresh_stale → engine.search), projects, ask_state (read_rollup / read_snapshot) | Contract lands only after its engine exists | Piped JSON-RPC session: `tools/call waggledance_search` returns marked snippets from 2 projects; `ask_state` returns live feature/phase | Agents query instead of re-reading |
| 3. Docs | README + PRD §5.5 describe the four-tool surface | Contract is public the moment it ships | Rendered docs page | Downstream agents discover the tools |

## Test matrix

Triad, smallest demonstrating size:
- Happy: `waggledance_search` over two registered projects returns hits from both with `<mark>`ed excerpts strictly wider than the old 12-token window (assert against a long document — a `≥12` assertion is green before the change and cannot fail); project filter narrows to one; `waggledance_projects` lists both with counts; `ask_state` unfiltered returns rollup rows, filtered returns snapshot with feature/phase/buckets.
- Edge: file modified on disk after last index → search reflects new content (D4 proof); file deleted → hit disappears; project root vanished → index survives untouched (refresh_stale guard b); empty/FTS-hostile query (`"*)("`) → empty result, no error; project with no `.bee/` → ask_state reports absent, not error.
- Error: unknown tool name → JSON-RPC `err()` `-32602` (not `tool_error`); missing required `query` arg → `tool_error` (`isError: true`); nonexistent project filter → `tool_error` naming the project.

Writers judge existing coverage first: `fts_search_finds_by_content_and_title` (repository.rs:736) and mcp.rs `viewable_text` tests stand; dispatch and refresh_stale tests are new.

## Out of scope

- `waggledance_context` digest/synthesis tool (deferred in CONTEXT.md).
- Web UI changes beyond inheriting the wider snippet; CLI changes; HTTP API.
- Watcher-driven or hash-based change detection (mtime+size chosen; revisit only on evidence of mtime unreliability).
