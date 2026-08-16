---
type: bee.delivery
title: waiting-on-badge — delivery
description: "Delivery record for work item waiting-on-badge: the kanban danger badge gated on a live waiting_on mark; run_state-only awaiting-approval reads Unreviewed."
timestamp: 2026-08-16
bee:
  id: waiting-on-badge-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/lanes/waiting-on-badge.json, .bee/cells/wob-1.json]
---

# waiting-on-badge — Delivery

## What shipped

- **wob-1** — Gate the Awaiting approval badge on a live waiting_on mark: `BeeState` parses `state.json`'s `waiting_on` into `waiting_on_live` (object with non-empty kind + subject; lenient mirror of bee's `waiting_on_is_live`), threaded to the hub card with the same active-feature gating as `run_state`; `run_state: awaiting-approval` renders the danger "Awaiting approval" chip only when the mark is live, and the neutral "Unreviewed" chip otherwise — bee derives awaiting-approval whenever any gate is pending with none later approved, and the user-invoked review gate routinely stays pending, so run_state alone must not claim a human is being waited on. Narrows kanban-live-signals D2; decision logged 2026-08-16. (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **wob-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (1064 passed)

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work waiting-on-badge` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/lanes/waiting-on-badge.json`. Applied 2026-08-16 from docs/history/waiting-on-badge/promote-proposals.md; proposal declared no area bullets and no pattern candidates.
