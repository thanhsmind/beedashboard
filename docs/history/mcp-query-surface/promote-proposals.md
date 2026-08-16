promote proposal for work item "mcp-query-surface" (docs/history/mcp-query-surface/CONTEXT.md + docs/history/mcp-query-surface/plan.md) — 3 capped cell(s): mqs-1, mqs-2, mqs-3
anchor: history — docs/history/mcp-query-surface/CONTEXT.md, docs/history/mcp-query-surface/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/mcp-query-surface/delivery.md

---
type: bee.delivery
title: mcp-query-surface — delivery
description: "Delivery record proposed by bee knowledge promote for work item mcp-query-surface: 3 capped cell(s), 0 recorded deviation(s)."
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

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **mqs-1** — `cargo test -p waggledance-core: stale matrix tests (modified/untouched/new/deleted/vanished-root) and the >12-token snippet test green alongside existing fts tests`
- **mqs-2** — `cargo test -p waggledance: dispatch tests green — tools/list has 4 schemas, three tool happy paths, err -32602 vs tool_error shapes asserted`
- **mqs-3** — `README MCP section and PRD §5.5 list the same four tools and argument names as tools/list in mcp.rs; cargo fmt --all --check && cargo clippy && cargo test workspace stays green`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work mcp-query-surface` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/mcp-query-surface/CONTEXT.md`, `docs/history/mcp-query-surface/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 3 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.