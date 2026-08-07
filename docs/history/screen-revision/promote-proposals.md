promote proposal for work item "screen-revision" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): sr-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/screen-revision/delivery.md

---
type: bee.delivery
title: screen-revision — delivery
description: "Delivery record proposed by bee knowledge promote for work item screen-revision: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-07
bee:
  id: screen-revision-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/sr-1.json]
---

# screen-revision — Delivery

## What shipped

- **sr-1** — Screen revision now hashes the rendered text (mdview_core::ansi::revision_of) instead of echoing herdr's own field, fixing terminal panes that froze on their first frame (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **sr-1** — `cargo test --workspace. New tests: (the defect) a pane whose output changed between two reads reports DIFFERENT revisions — this must fail against the current code, so write it first and see it red; (the guard) a pane whose output did not change reports the SAME revision, so the client dedupe still works and the page does not repaint on every tick; (edge) an empty screen reports a stable revision rather than panicking or reporting zero as a sentinel; (edge) two different panes with identical text do not collide in a way that suppresses either one's updates; (regression) the unassigned group's screen endpoint behaves the same way. Then confirm by eye against the live daemon on port 7788: open a pane whose output is moving and watch it actually move.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work screen-revision` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "screen-revision" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-07T03:42:56.195Z), the work item declares no bee.areas.

area agent-terminal:
  - [sr-1] Screen revision now hashes the rendered text (mdview_core::ansi::revision_of) instead of echoing herdr's own field, fixing terminal panes that froze on their first frame — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/sr-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.