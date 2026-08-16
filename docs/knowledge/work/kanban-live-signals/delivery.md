---
type: bee.delivery
title: kanban-live-signals — delivery
description: "Delivery record for work item kanban-live-signals: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: kanban-live-signals-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [docs/history/kanban-live-signals/CONTEXT.md, .bee/cells/archive/kanban-live-signals/kanban-live-signals-1.json, .bee/cells/archive/kanban-live-signals/kanban-live-signals-2.json]
---

# kanban-live-signals — Delivery

## What shipped

- **kanban-live-signals-1** — Added `state.json` `last_activity`/`run_state` fields, a bounded (64 KiB) `tools.jsonl` tail reader, and a `deferred-queue.jsonl` fold-by-id debt reader to the bee snapshot; the reader module's file-inventory doc updated to match.
- **kanban-live-signals-2** — Kanban cards render the merged last-activity clock (state stamp vs cell times), a ~2-minute "working now" pulse dot, a per-state `run_state` badge (awaiting-approval most prominent), and a per-feature deferred-debt count badge with hover detail. State-level signals appear only on the checkout's active feature's card; debt matches per entry's own feature.

## Verify

Each cell was capped only against a recorded passing verify result.

- **kanban-live-signals-1** — `cargo test --workspace` green; new reader tests: fields absent = None, tail window respected with torn first line and missing file, deferred-queue add-only vs resolved vs missing.
- **kanban-live-signals-2** — `cargo test --workspace` green (1000 passed); new card tests for activity merge, pulse window, run_state badge, deferred badge, active-feature scoping; old activity-line tests updated to the new contract, not deleted.

## Deviations

None recorded in the capped cell traces.

## Provenance

Mined by `bee knowledge promote --work kanban-live-signals` from the capped cell traces and docs/history/kanban-live-signals/CONTEXT.md; reviewed and applied at the 2026-08-16 compounding pass. Area sync for bee-cockpit landed at scribing ("Live signals on a card", docs/specs/bee-cockpit.md).
