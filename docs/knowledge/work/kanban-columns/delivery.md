---
type: bee.delivery
title: kanban-columns — delivery
description: "Delivery record for work item kanban-columns: the feature board grew from three columns to five, with only In Progress keeping cards."
timestamp: 2026-08-14
bee:
  id: kanban-columns-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [docs/history/kanban-columns/CONTEXT.md, docs/history/kanban-columns/plan.md]
  sources: [docs/history/kanban-columns/CONTEXT.md, docs/history/kanban-columns/plan.md, .bee/cells/kanban-columns-1.json, .bee/cells/kanban-columns-2.json]
---

# kanban-columns — Delivery

## What shipped

Both feature boards — the cross-project board on the home page's Kanban tab and
each project's own board — now show five columns instead of three: **Todo, In
Progress, Review, Compound, Finished**, in that left-to-right order. The former
"Waiting on you" column is gone; work stopped on an unapproved gate stays in In
Progress and says so on its own card.

Only In Progress renders cards. Todo, Review, Compound and Finished each render
one dense line per item and page at ten rows, the shape Finished already used.

- **kanban-columns-1** — Grew the classifier from three placements to five and
  rewrote its chain to test In Progress, Finished, Review, Compound, Todo in
  that order (D11). Narrowed what counts as live work for placement so that an
  unclaimed open cell no longer holds a feature in In Progress (D10) — without
  that narrowing Todo would never receive a feature. Deleted the Waiting
  placement, folding it into an In Progress card line (D5, D7, D8). Shared the
  dense row and its pager across all four row columns by parameterizing their
  group key and link (D12), and gave the grid one card-width track beside four
  narrower ones. Both boards change together because they share one classifier
  (D9). 2 files changed.
- **kanban-columns-2** — Backlog items in state `proposed` render as dense rows
  below Todo's features, linked to the owning project's bee board (D2, D3). On
  the cross-project board features from every project sit above proposed items
  from every project, which needs two accumulators per column rather than one.
  2 files changed.

## Behaviour that settled

- **What each column holds.** In Progress: live work — a claimed or stuck cell,
  or the active feature, or a live session naming it, or a granted worktree
  naming it. Finished: a closed feature, meaning a completed compounding phase
  or an archive of its own cells. Review: an unresolved review candidate names
  the feature. Compound: the feature is in its compounding phase. Todo:
  unclaimed open cells, plus proposed backlog items below them.
- **In Progress wins every tie.** A feature with live work stays in In Progress
  even when a review candidate is waiting on it; it reaches Review only once its
  live work is gone. The active feature never falls to Todo.
- **A closed feature stays closed.** Finished is tested before Review, so a
  feature that finished while still carrying an unresolved candidate reads as
  Finished, not as waiting for review.
- **Waiting is a line, not a place.** A feature stopped on an unapproved gate
  carries one line on its card: the label `Waiting on you` followed by the same
  reason text the old column used — the gate that stopped it, or the pause it
  is holding at.
- **A feature matching no column renders nowhere**, unchanged from before.

## Verify

Each cell was capped only against a recorded passing verify result — bee refuses
a cap without one.

- **kanban-columns-1** — `cargo test --workspace`
- **kanban-columns-2** — `cargo test --workspace`

## Deviations

- The feature ran in the main checkout rather than its own worktree. The write
  guard refuses writes into a sibling worktree from a session opened in main, and
  no other session was live at the time, so the worktree was unregistered and the
  deviation recorded.

## Open gaps

- Nothing records how a column behaves once it holds hundreds of rows beyond the
  first page; paging was carried over from Finished without re-testing at that
  size.
