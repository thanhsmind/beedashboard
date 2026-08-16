---
type: bee.delivery
title: mcp-query-surface — delivery
description: "Delivery record for work item mcp-query-surface: 3 capped cells shipping the four-tool agent-facing MCP query surface."
timestamp: 2026-08-16
bee:
  id: mcp-query-surface-delivery
  lifecycle: active
  required_context: [docs/history/mcp-query-surface/CONTEXT.md, docs/history/mcp-query-surface/plan.md]
  sources: [docs/history/mcp-query-surface/CONTEXT.md, docs/history/mcp-query-surface/plan.md, .bee/cells/mqs-1.json, .bee/cells/mqs-2.json, .bee/cells/mqs-3.json]
---

# mcp-query-surface — Delivery

## What shipped

- **mqs-1** — Engine::refresh_stale added with mtime/size-guarded selective re-index and delete-pass guards; search snippet window raised 12->64 tokens; SqliteStore sets a 5s busy_timeout on connection open (2 file(s) changed)
- **mqs-2** — Added waggledance_search/projects/ask_state MCP tools with D4 stale-refresh, D2 rich excerpts, and bee-state digests; match-based dispatch, 4 tool schemas, 15 new dispatch tests, all green (2 file(s) changed)
- **mqs-3** — Documented all four MCP tools (view_file, search, projects, ask_state) with shipped arg names/types in README's Agent integration section and PRD §5.5/§5.5.1 (2 file(s) changed)

The surface: `waggledance_search` (all registered projects by default, optional project filter, `<mark>`-highlighted 64-token snippets, stale-refresh of touched projects before answering), `waggledance_projects` (registry + file counts; counts reflect index as-is — recorded D4 narrowing), `waggledance_ask_state` (bee-state digest: rollup unfiltered, snapshot filtered; absent `.bee/` reports absent, not error), `waggledance_view_file` unchanged.

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **mqs-1** — `cargo test -p waggledance-core: stale matrix tests (modified/untouched/new/deleted/vanished-root) and the >12-token snippet test green alongside existing fts tests`
- **mqs-2** — `cargo test -p waggledance: dispatch tests green — tools/list has 4 schemas, three tool happy paths, err -32602 vs tool_error shapes asserted`
- **mqs-3** — `README MCP section and PRD §5.5 list the same four tools and argument names as tools/list in mcp.rs; cargo fmt --all --check && cargo clippy && cargo test workspace stays green`

Both behavior_change cells additionally carry an independent PASS judge verdict (model_independence: confirmed — builder sonnet, judge opus).

## Deviations

Corrected against the workers' Result forms — the mined trace field was empty, but two deviations were reported and verified:

- **mqs-2** — reformatted `crates/waggledance-core/src/engine.rs` via `cargo fmt --all` (formatting drift from mqs-1's commit blocked the workspace fmt gate); judge-verified formatting-only, one rustfmt trailing comma, zero semantic change.
- **mqs-3** — also updated README's summary-table "Agent-native" line (was "One MCP tool") for internal consistency; same file scope.

## Provenance

Proposed by `bee knowledge promote --work mcp-query-surface` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/mcp-query-surface/CONTEXT.md`, `docs/history/mcp-query-surface/plan.md`. Applied 2026-08-16 with the Deviations correction noted above.
