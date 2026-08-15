promote proposal for work item "no-input-zoom" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): no-input-zoom-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/no-input-zoom/delivery.md

---
type: bee.delivery
title: no-input-zoom — delivery
description: "Delivery record proposed by bee knowledge promote for work item no-input-zoom: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: no-input-zoom-delivery
  lifecycle: active
  areas: [appearance, agent-terminal, settings]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/no-input-zoom-1.json]
---

# no-input-zoom — Delivery

## What shipped

- **no-input-zoom-1** — Form fields rise to 16px on a touch pointer, so focusing the reply box or a settings field no longer zooms and sticks the layout magnified; desktop sizes and pinch-zoom are unchanged (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **no-input-zoom-1** — `cargo test --workspace green. New tests assert both halves survive: the app.css asset carries an @media (pointer: coarse) block setting .fg-input to 16px, and the terminal style block carries the same for .term-reply__text -- following the existing app.css assertion style at views.rs around line 5016.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work no-input-zoom` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "no-input-zoom" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-14T07:24:23.706Z), the work item declares no bee.areas.

area appearance:
  - [no-input-zoom-1] Form fields rise to 16px on a touch pointer, so focusing the reply box or a settings field no longer zooms and sticks the layout magnified; desktop sizes and pinch-zoom are unchanged — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/no-input-zoom-1.json)

area agent-terminal:
  - [no-input-zoom-1] Form fields rise to 16px on a touch pointer, so focusing the reply box or a settings field no longer zooms and sticks the layout magnified; desktop sizes and pinch-zoom are unchanged — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/no-input-zoom-1.json)

area settings:
  - [no-input-zoom-1] Form fields rise to 16px on a touch pointer, so focusing the reply box or a settings field no longer zooms and sticks the layout magnified; desktop sizes and pinch-zoom are unchanged — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/no-input-zoom-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 3 area bullet(s), 0 pattern candidate(s), 0 file(s) written.