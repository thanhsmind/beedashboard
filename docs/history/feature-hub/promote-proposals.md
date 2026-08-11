promote proposal for work item "feature-hub" (docs/history/feature-hub/CONTEXT.md + docs/history/feature-hub/plan.md) — 3 capped cell(s): feature-hub-1, feature-hub-2, feature-hub-3
anchor: history — docs/history/feature-hub/CONTEXT.md, docs/history/feature-hub/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/feature-hub/delivery.md

---
type: bee.delivery
title: feature-hub — delivery
description: "Delivery record proposed by bee knowledge promote for work item feature-hub: 3 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-11
bee:
  id: feature-hub-delivery
  lifecycle: active
  areas: [bee-cockpit-board]
  required_context: [docs/history/feature-hub/CONTEXT.md, docs/history/feature-hub/plan.md]
  sources: [docs/history/feature-hub/CONTEXT.md, docs/history/feature-hub/plan.md, .bee/cells/feature-hub-1.json, .bee/cells/feature-hub-2.json, .bee/cells/feature-hub-3.json]
---

# feature-hub — Delivery

## What shipped

- **feature-hub-1** — Replaced the Kanban cell board with a Waiting on you / In Progress / Finished grouped feature list and applied the anthropic.com-inspired palette; cargo test --workspace green (752 passed) (3 file(s) changed)
- **feature-hub-2** — Feature detail page restructured into Activity/Todos/Sub-agents tabs with a chip row (lane, worktree+merge state, duration, cell count); mdview-core gained BeeCell.outcome/tests, BeeLane.route and feature_cell_span as read-only joins over already-open files; cargo test --workspace green (758 passed). (3 file(s) changed)
- **feature-hub-3** — Fixed Finished predicate (compounding-complete/archive-dir OR, not dead terminal string), corrected worktree-chip Merged/Main rule to require workspace evidence, and added regression tests for F1-F5 plus mdview-core outcome-scrub coverage; cargo test --workspace green (763 passed) (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **feature-hub-1** — `cargo test --workspace`
- **feature-hub-2** — `cargo test --workspace`
- **feature-hub-3** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work feature-hub` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/feature-hub/CONTEXT.md`, `docs/history/feature-hub/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "feature-hub" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-11T12:18:33.654Z), the work item declares no bee.areas.

area bee-cockpit-board:
  - [feature-hub-1] Replaced the Kanban cell board with a Waiting on you / In Progress / Finished grouped feature list and applied the anthropic.com-inspired palette; cargo test --workspace green (752 passed) — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/feature-hub-1.json)
  - [feature-hub-2] Feature detail page restructured into Activity/Todos/Sub-agents tabs with a chip row (lane, worktree+merge state, duration, cell count); mdview-core gained BeeCell.outcome/tests, BeeLane.route and feature_cell_span as read-only joins over already-open files; cargo test --workspace green (758 passed). — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/feature-hub-2.json)
  - [feature-hub-3] Fixed Finished predicate (compounding-complete/archive-dir OR, not dead terminal string), corrected worktree-chip Merged/Main rule to require workspace evidence, and added regression tests for F1-F5 plus mdview-core outcome-scrub coverage; cargo test --workspace green (763 passed) — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/feature-hub-3.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 3 capped cell(s) mined, 1 delivery draft, 3 area bullet(s), 0 pattern candidate(s), 0 file(s) written.