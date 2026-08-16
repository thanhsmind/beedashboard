promote proposal for work item "mcp-query-surface" (docs/history/mcp-query-surface/CONTEXT.md + docs/history/mcp-query-surface/plan.md) — 5 capped cell(s): mqs-1, mqs-2, mqs-3, mqs-4, mqs-5
anchor: history — docs/history/mcp-query-surface/CONTEXT.md, docs/history/mcp-query-surface/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/mcp-query-surface/delivery.md

---
type: bee.delivery
title: mcp-query-surface — delivery
description: "Delivery record proposed by bee knowledge promote for work item mcp-query-surface: 5 capped cell(s), 0 recorded deviation(s)."
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
- **mqs-4** — refresh_stale now deletes only stat-confirmed-NotFound rows; gitignored/excluded indexed files and permission-denied stats survive (1 file(s) changed)
- **mqs-5** — handle_search now reports refresh outcome (refreshed/failed) in structuredContent.refresh and appends a warning line on failure; docs updated from unconditional freshness promise to reflect it (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **mqs-1** — `cargo test -p waggledance-core: stale matrix tests (modified/untouched/new/deleted/vanished-root) and the >12-token snippet test green alongside existing fts tests`
- **mqs-2** — `cargo test -p waggledance: dispatch tests green — tools/list has 4 schemas, three tool happy paths, err -32602 vs tool_error shapes asserted`
- **mqs-3** — `README MCP section and PRD §5.5 list the same four tools and argument names as tools/list in mcp.rs; cargo fmt --all --check && cargo clippy && cargo test workspace stays green`
- **mqs-4** — `cargo test -p waggledance-core: gitignored-indexed file survives refresh_stale; permission-denied stat keeps the row; truly-deleted file still removed; existing stale matrix stays green`
- **mqs-5** — `cargo test -p waggledance: search response carries a refresh outcome field; happy path reports failed=[]; a provable failure path (test seam or induced store error) lands in failed with the project id; README/PRD wording no longer promises unconditional freshness`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work mcp-query-surface` from 5 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/mcp-query-surface/CONTEXT.md`, `docs/history/mcp-query-surface/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "mcp-query-surface" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-16T11:16:39.483Z), the work item declares no bee.areas.

area mcp-surface:
  - [mqs-1] Engine::refresh_stale added with mtime/size-guarded selective re-index and delete-pass guards; search snippet window raised 12->64 tokens; SqliteStore sets a 5s busy_timeout on connection open — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/archive/mcp-query-surface/mqs-1.json)
  - [mqs-2] Added waggledance_search/projects/ask_state MCP tools with D4 stale-refresh, D2 rich excerpts, and bee-state digests; match-based dispatch, 4 tool schemas, 15 new dispatch tests, all green — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/archive/mcp-query-surface/mqs-2.json)
  - [mqs-4] refresh_stale now deletes only stat-confirmed-NotFound rows; gitignored/excluded indexed files and permission-denied stats survive — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/mqs-4.json)
  - [mqs-5] handle_search now reports refresh outcome (refreshed/failed) in structuredContent.refresh and appends a warning line on failure; docs updated from unconditional freshness promise to reflect it — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/mqs-5.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

from cell mqs-5 — save as docs/knowledge/patterns/mcp-query-surface-mqs-5-pitfall.md

---
type: bee.pattern
title: mcp-query-surface cell mqs-5 — pitfall candidate
description: "Pitfall candidate mined from cell mqs-5's capped trace: c6f5bbe4cc40"
timestamp: 2026-08-16
bee:
  id: mcp-query-surface-mqs-5-pitfall
  lifecycle: draft
  areas: [mcp-surface]
  sources: [.bee/cells/mqs-5.json]
  polarity: pitfall
---

# mcp-query-surface cell mqs-5 — pitfall candidate

## What the cell did

handle_search now reports refresh outcome (refreshed/failed) in structuredContent.refresh and appends a warning line on failure; docs updated from unconditional freshness promise to reflect it

## Recorded evidence (verbatim from .bee/cells/mqs-5.json)

- **failure_signature** — c6f5bbe4cc40

## Status

Candidate only. `bee knowledge promote` proposes; naming the pattern, generalizing it beyond this cell, and moving `bee.lifecycle` to `active` are a human or agent decision.

knowledge promote: 5 capped cell(s) mined, 1 delivery draft, 4 area bullet(s), 1 pattern candidate(s), 0 file(s) written.