# Poller In-Flight Guard — Context (small brief)

**Feature slug:** poller-inflight-guard · 2026-08-16 · small · bugfix

## Asked / found

Backlog #30: the app.js screen poller (`pollOne`/`pollAll`, ~assets/app.js:1071-1134)
has no in-flight guard; every 1.5s tick fires a fetch per pane even if the previous
one is still outstanding, and each fetch takes the per-pane async mutex (`pane_lock`)
server-side — a herdr slower than 1500ms queues ticks unbounded. The transcript
poller already solved this with an `inFlight[paneId]` check/set/clear
(~assets/app.js:1359-1449).

## Locked Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D1 | Give the screen poller the same in-flight guard the transcript poller uses: a pane whose screen fetch is still outstanding is skipped on the next tick instead of stacking a second fetch. Clear the flag on completion (success and error). | Decision log 6e77a72a. Mirrors the proven transcript-poller pattern; no new mechanism. |

### Agent's Discretion
- Reuse the transcript poller's exact `inFlight` map or a screen-specific one — whichever reads cleaner; do not share one map across the two pollers if their pane-id spaces differ.

## Verify
JS predicate, no repo harness. Proof: a Rust boundary test is not meaningful here
(pure client timing). Record the JS-only guard the way home-terminal-header-2 did:
manual browser check — on a project terminal page with a slow/hung pane, only one
screen fetch is outstanding at a time (network panel shows no stacking). Keep
`cargo test --workspace` green.

## Handoff
One cell, one worker, one worktree.
