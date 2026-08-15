promote proposal for work item "kanban-live-signals" (docs/history/kanban-live-signals/CONTEXT.md) — 2 capped cell(s): kanban-live-signals-1, kanban-live-signals-2
anchor: history — docs/history/kanban-live-signals/CONTEXT.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/kanban-live-signals/delivery.md

---
type: bee.delivery
title: kanban-live-signals — delivery
description: "Delivery record proposed by bee knowledge promote for work item kanban-live-signals: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: kanban-live-signals-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/history/kanban-live-signals/CONTEXT.md]
  sources: [docs/history/kanban-live-signals/CONTEXT.md, .bee/cells/kanban-live-signals-1.json, .bee/cells/kanban-live-signals-2.json]
---

# kanban-live-signals — Delivery

## What shipped

- **kanban-live-signals-1** — Added state.json last_activity/run_state fields, a bounded tools.jsonl tail reader, and a deferred-queue debt reader to the bee snapshot (1 file(s) changed)
- **kanban-live-signals-2** — Show activity, run_state badge, and deferred debt on kanban cards (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **kanban-live-signals-1** — `cargo test --workspace green. New unit tests beside the module's existing reader tests: state.json with and without last_activity/run_state (absent = None); a tools.jsonl fixture larger than the tail window proving only the tail is parsed and the newest ts wins, plus a torn first line and a missing file; a deferred-queue fixture with adds only (all unresolved), an add followed by another event for the same id (resolved), and a missing file (zero debt).`
- **kanban-live-signals-2** — `cargo test --workspace green. New view tests beside the existing bee_hub_card tests: last-activity picks state.json last_activity when newer than cell timestamps and falls back when absent; a snapshot with a recent tool call renders the pulse marker and an old one does not; run_state awaiting-approval renders its prominent badge and absent run_state renders none; deferred debt count renders with its detail and zero debt renders nothing. Any existing test asserting the old claimed_at/capped_at-only activity line is updated to the new contract, not deleted.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work kanban-live-signals` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/kanban-live-signals/CONTEXT.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "kanban-live-signals" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-15T16:13:37.236Z), the work item declares no bee.areas.

area bee-cockpit:
  - [kanban-live-signals-1] Added state.json last_activity/run_state fields, a bounded tools.jsonl tail reader, and a deferred-queue debt reader to the bee snapshot — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/kanban-live-signals-1.json)
  - [kanban-live-signals-2] Show activity, run_state badge, and deferred debt on kanban cards — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/kanban-live-signals-2.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 2 area bullet(s), 0 pattern candidate(s), 0 file(s) written.