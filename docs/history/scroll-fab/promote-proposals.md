promote proposal for work item "scroll-fab" (.bee/logs/scribing-runs.jsonl + .bee/lanes/scroll-fab.json) — 1 capped cell(s): scroll-fab-1
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/scroll-fab.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/scroll-fab/delivery.md

---
type: bee.delivery
title: scroll-fab — delivery
description: "Delivery record proposed by bee knowledge promote for work item scroll-fab: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-12
bee:
  id: scroll-fab-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/scroll-fab.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/scroll-fab.json, .bee/cells/scroll-fab-1.json]
---

# scroll-fab — Delivery

## What shipped

- **scroll-fab-1** — Reworked pane scroll controls into a round Older/Newer/Live column with Newer wired to the shared depth path (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **scroll-fab-1** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work scroll-fab` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/scroll-fab.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "scroll-fab" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-12T05:35:30.733Z), the work item declares no bee.areas.

area agent-terminal:
  - [scroll-fab-1] Reworked pane scroll controls into a round Older/Newer/Live column with Newer wired to the shared depth path — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/scroll-fab-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.