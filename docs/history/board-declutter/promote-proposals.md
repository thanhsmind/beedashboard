promote proposal for work item "board-declutter" (.bee/logs/scribing-runs.jsonl + .bee/lanes/board-declutter.json) — 1 capped cell(s): board-declutter-1
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/board-declutter.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/board-declutter/delivery.md

---
type: bee.delivery
title: board-declutter — delivery
description: "Delivery record proposed by bee knowledge promote for work item board-declutter: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-11
bee:
  id: board-declutter-delivery
  lifecycle: active
  areas: [bee-cockpit-board]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/board-declutter.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/board-declutter.json, .bee/cells/board-declutter-1.json]
---

# board-declutter — Delivery

## What shipped

- **board-declutter-1** — Removed stepper/KPI/velocity/attention/working-now from the bee board page; Feature Hub is now the first main section; retired/rewrote board-page tests that asserted the removed markup and added a hub-first regression test (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **board-declutter-1** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work board-declutter` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/board-declutter.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "board-declutter" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-11T13:14:36.012Z), the work item declares no bee.areas.

area bee-cockpit-board:
  - [board-declutter-1] Removed stepper/KPI/velocity/attention/working-now from the bee board page; Feature Hub is now the first main section; retired/rewrote board-page tests that asserted the removed markup and added a hub-first regression test — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/board-declutter-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.