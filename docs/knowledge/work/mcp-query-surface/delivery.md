---
type: bee.delivery
title: mcp-query-surface — delivery
description: "Delivery record for work item mcp-query-surface: 5 capped cells shipping the four-tool agent-facing MCP query surface plus its review P1 fixes."
timestamp: 2026-08-16
bee:
  id: mcp-query-surface-delivery
  lifecycle: active
  areas: [mcp-surface]
  required_context: [docs/history/mcp-query-surface/CONTEXT.md, docs/history/mcp-query-surface/plan.md]
  sources: [docs/history/mcp-query-surface/CONTEXT.md, docs/history/mcp-query-surface/plan.md, .bee/cells/archive/mcp-query-surface/mqs-1.json, .bee/cells/archive/mcp-query-surface/mqs-2.json, .bee/cells/archive/mcp-query-surface/mqs-3.json, .bee/cells/mqs-4.json, .bee/cells/mqs-5.json]
---

# mcp-query-surface — Delivery

## What shipped

- **mqs-1** — Engine::refresh_stale added with mtime/size-guarded selective re-index and delete-pass guards; search snippet window raised 12->64 tokens; SqliteStore sets a 5s busy_timeout on connection open (2 file(s) changed)
- **mqs-2** — Added waggledance_search/projects/ask_state MCP tools with D4 stale-refresh, D2 rich excerpts, and bee-state digests; match-based dispatch, 4 tool schemas, 15 new dispatch tests, all green (2 file(s) changed)
- **mqs-3** — Documented all four MCP tools (view_file, search, projects, ask_state) with shipped arg names/types in README's Agent integration section and PRD §5.5/§5.5.1 (2 file(s) changed)
- **mqs-4** — refresh_stale deletes only stat-confirmed-NotFound rows; gitignored/excluded indexed files and permission-denied stats survive the delete pass (1 file(s) changed)
- **mqs-5** — handle_search reports the refresh outcome (refreshed/failed per project) in structuredContent.refresh and appends a visible warning line on failure; docs updated from an unconditional freshness promise to reflect it (3 file(s) changed)

The surface: `waggledance_search` (all registered projects by default, optional project filter, `<mark>`-highlighted 64-token snippets, stale-refresh of touched projects before answering, refresh outcome reported per project with a warning on failure), `waggledance_projects` (registry + file counts; counts reflect index as-is — recorded D4 narrowing), `waggledance_ask_state` (bee-state digest: rollup unfiltered, snapshot filtered; absent `.bee/` reports absent, not error), `waggledance_view_file` unchanged. Index rows are removed only when a file's stat returns NotFound — walk absence, gitignore, or stat errors never delete.

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **mqs-1** — `cargo test -p waggledance-core: stale matrix tests (modified/untouched/new/deleted/vanished-root) and the >12-token snippet test green alongside existing fts tests`
- **mqs-2** — `cargo test -p waggledance: dispatch tests green — tools/list has 4 schemas, three tool happy paths, err -32602 vs tool_error shapes asserted`
- **mqs-3** — `README MCP section and PRD §5.5 list the same four tools and argument names as tools/list in mcp.rs; cargo fmt --all --check && cargo clippy && cargo test workspace stays green`
- **mqs-4** — `cargo test -p waggledance-core: gitignored-indexed file survives refresh_stale; permission-denied stat keeps the row; truly-deleted file still removed; existing stale matrix stays green`
- **mqs-5** — `cargo test -p waggledance: search response carries a refresh outcome field; happy path reports failed=[]; a provable failure path lands in failed with the project id; README/PRD wording no longer promises unconditional freshness`

Both behavior_change cells additionally carry an independent PASS judge verdict (model_independence: confirmed — builder sonnet, judge opus).

## Deviations

Corrected against the workers' Result forms — the mined trace field was empty, but two deviations were reported and verified:

- **mqs-2** — reformatted `crates/waggledance-core/src/engine.rs` via `cargo fmt --all` (formatting drift from mqs-1's commit blocked the workspace fmt gate); judge-verified formatting-only, one rustfmt trailing comma, zero semantic change.
- **mqs-3** — also updated README's summary-table "Agent-native" line (was "One MCP tool") for internal consistency; same file scope.

## Provenance

Proposed by `bee knowledge promote --work mcp-query-surface` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/mcp-query-surface/CONTEXT.md`, `docs/history/mcp-query-surface/plan.md`. Applied 2026-08-16 with the Deviations correction noted above.
