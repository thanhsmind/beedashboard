promote proposal for work item "doctor-preserve-order" (.bee/logs/scribing-runs.jsonl + .bee/lanes/doctor-preserve-order.json + docs/history/doctor-preserve-order/promote-proposals.md) — 1 capped cell(s): dpo-1
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/doctor-preserve-order.json, docs/history/doctor-preserve-order/promote-proposals.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/doctor-preserve-order/delivery.md

---
type: bee.delivery
title: doctor-preserve-order — delivery
description: "Delivery record proposed by bee knowledge promote for work item doctor-preserve-order: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-20
bee:
  id: doctor-preserve-order-delivery
  lifecycle: active
  areas: [doctor]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/doctor-preserve-order.json, docs/history/doctor-preserve-order/promote-proposals.md]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/doctor-preserve-order.json, docs/history/doctor-preserve-order/promote-proposals.md, .bee/cells/dpo-1.json]
---

# doctor-preserve-order — Delivery

## What shipped

- **dpo-1** — serde_json preserve_order enabled; doctor --fix no longer reorders ~/.claude.json keys (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **dpo-1** — `cargo test -p waggledance doctor`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work doctor-preserve-order` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/doctor-preserve-order.json`, `docs/history/doctor-preserve-order/promote-proposals.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "doctor-preserve-order" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-20T09:56:00.069Z), the work item declares no bee.areas.

area doctor:
  - [dpo-1] serde_json preserve_order enabled; doctor --fix no longer reorders ~/.claude.json keys — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/dpo-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.