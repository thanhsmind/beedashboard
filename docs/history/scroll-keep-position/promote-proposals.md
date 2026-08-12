promote proposal for work item "scroll-keep-position" (.bee/logs/scribing-runs.jsonl + .bee/lanes/scroll-keep-position.json) — 3 capped cell(s): scroll-keep-position-1, scroll-keep-position-2, scroll-keep-position-3
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/scroll-keep-position.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/scroll-keep-position/delivery.md

---
type: bee.delivery
title: scroll-keep-position — delivery
description: "Delivery record proposed by bee knowledge promote for work item scroll-keep-position: 3 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-12
bee:
  id: scroll-keep-position-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/scroll-keep-position.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/scroll-keep-position.json, .bee/cells/scroll-keep-position-1.json, .bee/cells/scroll-keep-position-2.json, .bee/cells/scroll-keep-position-3.json]
---

# scroll-keep-position — Delivery

## What shipped

- **scroll-keep-position-1** — Made pane scroll depth stateful: PaneScroller::read_to_depth moves only the delta, AppState::scroll_tracker records per-pane depth with idle-TTL/live-restore/content-mismatch rails, Live button sends explicit history=0 (3 file(s) changed)
- **scroll-keep-position-2** — Normalised the mismatch comparison, fixed the depth-0 restore and idle-sweep gaps, serialised per-pane scroll ops, and added the five+ required proofs (3 file(s) changed)
- **scroll-keep-position-3** — Fixed the depth-0 payload shape, both failure-safe record clears, and the settle-wait short-circuit, with tests for all four; cargo test --workspace: 788 passed (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **scroll-keep-position-1** — `cargo test --workspace`
- **scroll-keep-position-2** — `cargo test --workspace`
- **scroll-keep-position-3** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work scroll-keep-position` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/scroll-keep-position.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "scroll-keep-position" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-12T05:03:39.476Z), the work item declares no bee.areas.

area agent-terminal:
  - [scroll-keep-position-1] Made pane scroll depth stateful: PaneScroller::read_to_depth moves only the delta, AppState::scroll_tracker records per-pane depth with idle-TTL/live-restore/content-mismatch rails, Live button sends explicit history=0 — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/scroll-keep-position-1.json)
  - [scroll-keep-position-2] Normalised the mismatch comparison, fixed the depth-0 restore and idle-sweep gaps, serialised per-pane scroll ops, and added the five+ required proofs — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/scroll-keep-position-2.json)
  - [scroll-keep-position-3] Fixed the depth-0 payload shape, both failure-safe record clears, and the settle-wait short-circuit, with tests for all four; cargo test --workspace: 788 passed — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/scroll-keep-position-3.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 3 capped cell(s) mined, 1 delivery draft, 3 area bullet(s), 0 pattern candidate(s), 0 file(s) written.