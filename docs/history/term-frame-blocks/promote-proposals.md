promote proposal for work item "term-frame-blocks" (.bee/logs/scribing-runs.jsonl + .bee/lanes/term-frame-blocks.json) — 1 capped cell(s): term-frame-blocks-1
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/term-frame-blocks.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/term-frame-blocks/delivery.md

---
type: bee.delivery
title: term-frame-blocks — delivery
description: "Delivery record proposed by bee knowledge promote for work item term-frame-blocks: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-12
bee:
  id: term-frame-blocks-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/term-frame-blocks.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/term-frame-blocks.json, .bee/cells/term-frame-blocks-1.json]
---

# term-frame-blocks — Delivery

## What shipped

- **term-frame-blocks-1** — Wrapped box-drawing frame runs in .term-frame divs, closing/reopening SGR spans across the boundary, so tables and TUI frames keep their grid on phones while prose still wraps (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **term-frame-blocks-1** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work term-frame-blocks` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/term-frame-blocks.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "term-frame-blocks" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-12T01:17:02.555Z), the work item declares no bee.areas.

area agent-terminal:
  - [term-frame-blocks-1] Wrapped box-drawing frame runs in .term-frame divs, closing/reopening SGR spans across the boundary, so tables and TUI frames keep their grid on phones while prose still wraps — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/term-frame-blocks-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.