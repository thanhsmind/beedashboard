---
type: bee.delivery
title: cross-board — delivery
description: "Delivery record proposed by bee knowledge promote for work item cross-board: 3 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-12
bee:
  id: cross-board-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [docs/history/cross-board/CONTEXT.md, docs/history/cross-board/plan.md]
  sources: [docs/history/cross-board/CONTEXT.md, docs/history/cross-board/plan.md, .bee/cells/archive/cross-board/cross-board-1.json, .bee/cells/archive/cross-board/cross-board-2.json, .bee/cells/archive/cross-board/cross-board-3.json]
---

# cross-board — Delivery

## What shipped

The viewer's front page became a roll-up of bee state across every registered
project that has a `.bee/` store: a cross-project Live strip, a cross-project
Features board with the same columns the per-project board uses — three at the
time, five since kanban-columns — and the existing project list beneath them,
unchanged.

- **cross-board-1** — Added `read_rollup`: a synchronous multi-project roll-up
  returning a per-root `BeeSnapshot` plus each archived feature's ship time,
  taken as the latest `trace.capped_at` across that feature's archived cells
  and absent when any of them lacks one (D10). 1 file changed.
- **cross-board-2** — Split `bee_feature_hub_section` into a classification step
  returning data and a render step, leaving the per-project board's output
  unchanged; added `bee_cross_project_features_section`, which merges the
  per-project columns flat, labels every card and row with its project (D5), and
  orders, counts and caps the merged Finished sequence (D10, D7). 1 file changed.
- **cross-board-3** — The home page now composes a cross-project Live strip and
  Features board above the unchanged project list (D1), gated on the existing
  `.bee/` qualification (D8) and absent entirely when nothing qualifies (D9); the
  roll-up runs off the async task, one `spawn_blocking` task per qualifying
  project, spawned concurrently. 2 files changed.

## Verify

Each cell was capped only against a recorded passing verify result — bee refuses
a cap without one. The declared suite is `cargo test --workspace`; it went from
827 passing before the feature to 839 after, with the twelve `home_page_*` router
tests and twelve of the sixteen hub unit tests unedited throughout.

- **cross-board-1** — roll-up over two roots returns one snapshot each in the
  order given; an all-timed feature reports the latest `capped_at`; a
  mixed-timed feature reports no ship time; a root with no archive yields an
  empty set rather than an error; an unparseable archived cell does not lose its
  siblings. The framework-free guard `no_web_framework_dependency_declared`
  stayed green and unedited.
- **cross-board-2** — the sixteen existing hub unit tests stayed green (twelve
  unedited; four updated only to pass the new renderer arguments) and the twelve
  `feature_hub_*` router tests stayed green and unedited. New cases cover
  placement and labelling across projects, D10 merge ordering, D7 paging on the
  merged total, the same feature slug owned by two projects, and a project
  contributing nothing.
- **cross-board-3** — the twelve `home_page_*` router tests stayed green and
  unedited, including the script-selector test. New cases cover several
  qualifying projects, no qualifying project, a registered root that no longer
  exists on disk, and a project whose `.bee/` holds a corrupt cell still leaving
  the page at 200. The off-thread requirement is proved structurally at the call
  site rather than by a timeout, because a timeout around `spawn_blocking`
  abandons the thread instead of stopping the read.

Confirmed live after install: `/` answered 200 in 0.27 s over eight qualifying
projects, showing Live, then Features with Waiting 0 / In Progress 3 /
Finished 164, then the project list.

## Deviations

None recorded in the capped cell traces.

Two workflow deviations were recorded outside the traces, both as decisions:
the feature was built in the main checkout rather than its own worktree, and
`plan.md` carries no approval stamp because stamping requires a lane record this
feature never had.

## Provenance

Proposed by `bee knowledge promote --work cross-board` from 3 capped cell traces
and the anchor `docs/history/cross-board/CONTEXT.md`. The proposal's area-update
bullets were reviewed and not applied — see the reason recorded in the decision
log; the behaviour they describe was merged into `docs/specs/bee-cockpit.md` and
`docs/specs/web-interface.md` in business language instead.
