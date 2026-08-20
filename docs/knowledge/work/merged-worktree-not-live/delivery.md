---
type: bee.delivery
title: merged-worktree-not-live — delivery
description: "Delivery record for work item merged-worktree-not-live: a feature stops looking busy once its work has landed."
timestamp: 2026-08-18
bee:
  id: merged-worktree-not-live-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/lanes/merged-worktree-not-live.json, .bee/cells/archive/merged-worktree-not-live/merged-worktree-not-live-1.json, .bee/cells/archive/merged-worktree-not-live/merged-worktree-not-live-2.json]
---

# merged-worktree-not-live — Delivery

## What shipped

Finished features kept advertising themselves as in flight. Two separate
signals were reading stale, and the board believed both.

**A workspace that has already been merged no longer counts as live work.**
When a feature's separate workspace is merged but deliberately kept on disk,
the merge leaves a cleanup task queued rather than removing it. The board
used to see the surviving workspace and pin the feature under In Progress
forever. It now reads the same cleanup queue the tooling itself reads: a
workspace whose cleanup was queued and never completed is merged-but-kept,
and is excluded from the live-work test. Cleanup completed, or never queued,
and the workspace counts as live again.

**Finished means every terminal state, not one.** The board tested for a
single end-of-life phase name. Closing a feature had since begun writing a
second one, so genuinely finished features sat in the wrong column. The test
now accepts the whole terminal set — a closed feature and a fully compounded
one both read as finished — while a feature still holding work in progress
stays In Progress regardless of the phase it names.

Both signals are read as sets rather than single values: one surviving
artifact is not proof of live work, and one phase name is not the only way
to be done.

## Verify

`cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D
warnings && cargo test --workspace` green on both cells. New cases cover a
closed-phase feature landing under Finished, the same feature staying In
Progress while a cell is still doing, a bound session with no cells still
reading Finished, and a merged-but-kept workspace no longer pinning its
feature to In Progress — asserted at the view layer and again over HTTP.
Every pre-existing case for the older phase name was left untouched and
green.

## Deviations

None recorded.

## Pointers

- `crates/waggledance-core/src/bee.rs:626` — `BeeWorktree.merged_pending`;
  derived by `read_merged_pending_worktrees` at `bee.rs:2570` from
  `.bee/deferred-queue.jsonl` (an `add` of kind `worktree-cleanup` with no
  later `complete` for the same id), applied at `bee.rs:2534-2540`.
- `crates/waggledance/src/views.rs:2658` — the `worktree_bound` filter that
  drops merged-pending worktrees out of `has_live_work`.
- `crates/waggledance/src/views.rs:2613-2615` — `bee_phase_is_terminal`
  (`idle` or `compounding-complete`), consumed by `is_finished` at
  `views.rs:2696`.
- Tests: `views.rs:9098`, `views.rs:9169`, `views.rs:9242`,
  `server.rs:5555`, `server.rs:5605`, `server.rs:5814`.

## Provenance

Written from the capped traces of `merged-worktree-not-live-1` and
`merged-worktree-not-live-2`, verified against the shipped source. The
set-not-single-value rule is the decision logged 2026-08-18.
