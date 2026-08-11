promote proposal for work item "archive-visibility" (.bee/logs/scribing-runs.jsonl + .bee/lanes/archive-visibility.json) — 1 capped cell(s): archive-visibility-1
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/archive-visibility.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/archive-visibility/delivery.md

---
type: bee.delivery
title: archive-visibility — delivery
description: "Delivery record proposed by bee knowledge promote for work item archive-visibility: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-11
bee:
  id: archive-visibility-delivery
  lifecycle: active
  areas: [bee-cockpit-board]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/archive-visibility.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/archive-visibility.json, .bee/cells/archive-visibility-1.json]
---

# archive-visibility — Delivery

## What shipped

- **archive-visibility-1** — Feature and cell detail pages now merge archived cells (Closed header with done count); main board KPIs/buckets stay archive-free (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **archive-visibility-1** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work archive-visibility` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/archive-visibility.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "archive-visibility" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-11T10:24:17.034Z), the work item declares no bee.areas.

area bee-cockpit-board:
  - [archive-visibility-1] Feature and cell detail pages now merge archived cells (Closed header with done count); main board KPIs/buckets stay archive-free — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/archive-visibility-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.