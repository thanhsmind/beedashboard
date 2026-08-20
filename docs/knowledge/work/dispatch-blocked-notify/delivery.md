---
type: bee.delivery
title: dispatch-blocked-notify — delivery
description: "Delivery record for work item dispatch-blocked-notify: 5 capped cells giving a dispatched run its own alert when it reaches a status only a human can clear."
timestamp: 2026-08-20
bee:
  id: dispatch-blocked-notify-delivery
  lifecycle: active
  areas: [orchestration, notifications]
  required_context: [docs/history/dispatch-blocked-notify/CONTEXT.md]
  sources: [docs/history/dispatch-blocked-notify/CONTEXT.md, .bee/cells/dbn-1.json, .bee/cells/dbn-2.json, .bee/cells/dbn-3.json, .bee/cells/dbn-4.json, .bee/cells/dbn-5.json]
---

# dispatch-blocked-notify — Delivery

The work shape lives on the feature branch as `docs/history/dispatch-blocked-notify/plan.md` and lands here with the merge.

## What shipped

- **dbn-1** — The notification outbox carries run and project identity, migrates databases that already exist, and holds one row per run per status through a uniqueness constraint (dispatch-blocked-notify D4, dispatch-blocked-notify D5).
- **dbn-2** — A run reaching a status only a human can clear raises exactly one alert, from the single point where that transition is already persisted; the body names project, pane and run and nothing else (dispatch-blocked-notify D1, dispatch-blocked-notify D2, dispatch-blocked-notify D4).
- **dbn-3** — The notification store is armed and disarmed by the existing opt-in switch (dispatch-blocked-notify D6).
- **dbn-4** — The await path receives a real store opened against the same database the delivery drain reads, so the alert is live rather than inert; an enqueue failure now leaves a warning instead of vanishing (dispatch-blocked-notify D1, dispatch-blocked-notify D6).
- **dbn-5** — While a dispatched run owns a pane, the older pane-status alert for that pane stays silent, so one event reaches the human once (dispatch-blocked-notify D3).

## How it was verified

- `cargo test -p waggledance-core notify_store` — 9 passed (dbn-1).
- `cargo test -p waggledance orchestrate` — 17 passed (dbn-2).
- `cargo test -p waggledance reconcile` — green (dbn-3).
- `cargo test -p waggledance mcp` — green (dbn-4).
- `cargo test -p waggledance notify` — green (dbn-5).
- Full declared command over the finished branch: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — clean, 1157 passed.

## Deviations worth keeping

- **dbn-3 shipped an inert link.** It added an accessor nothing outside its own tests called, while the real raise point still received nothing — and the two lived in different processes, so handing the store across could never have worked. Three green cells still added up to a dead feature. The repair was dbn-4, and the lesson is that a cell claiming a user-visible outcome owes one proof that runs the whole path, not three that each prove a segment.
- **Workers cap what they can reach.** An external pane worker committed and capped from the feature worktree but could not write the dispatch mailbox result, which lives in the main checkout; another capped without running the formatter, leaving the branch red for CI until the orchestrator formatted it.
- **A pane-ownership lookup scans every project.** `is_pane_owned_by_run` walks all projects and up to fifty runs each, on every watcher poll. Correct today at this data size, and the obvious thing to index if run volume grows.
