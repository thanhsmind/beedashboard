promote proposal for work item "feature-titles" (.bee/logs/scribing-runs.jsonl + .bee/lanes/feature-titles.json) — 2 capped cell(s): feature-titles-1, feature-titles-2
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/feature-titles.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/feature-titles/delivery.md

---
type: bee.delivery
title: feature-titles — delivery
description: "Delivery record proposed by bee knowledge promote for work item feature-titles: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-11
bee:
  id: feature-titles-delivery
  lifecycle: active
  areas: [bee-cockpit-board]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/feature-titles.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/feature-titles.json, .bee/cells/feature-titles-1.json, .bee/cells/feature-titles-2.json]
---

# feature-titles — Delivery

## What shipped

- **feature-titles-1** — Human titles + descriptions from CONTEXT.md; docs links on feature detail (3 file(s) changed)
- **feature-titles-2** — Replaced the feature detail Sub-agents tab with a Terminal tab listing/linking the project's live agent-terminal panes (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **feature-titles-1** — `cargo test --workspace`
- **feature-titles-2** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work feature-titles` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/feature-titles.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "feature-titles" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-11T15:24:00.862Z), the work item declares no bee.areas.

area bee-cockpit-board:
  - [feature-titles-1] Human titles + descriptions from CONTEXT.md; docs links on feature detail — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/feature-titles-1.json)
  - [feature-titles-2] Replaced the feature detail Sub-agents tab with a Terminal tab listing/linking the project's live agent-terminal panes — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/feature-titles-2.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 2 area bullet(s), 0 pattern candidate(s), 0 file(s) written.