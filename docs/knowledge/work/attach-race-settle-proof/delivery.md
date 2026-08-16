---
type: bee.delivery
title: attach-race-settle-proof — delivery
description: "Delivery record for work item attach-race-settle-proof: the settle wait's three mechanisms are now mutation-caught — deleting any of them turns the suite red."
timestamp: 2026-08-16
bee:
  id: attach-race-settle-proof-delivery
  lifecycle: active
  required_context: [docs/history/learnings/20260816-a-lower-bound-assertion-proves-nothing.md]
  sources: [.bee/lanes/attach-race-settle-proof.json, .bee/cells/archive/attach-race-settle-proof/attach-race-settle-proof-1.json, .bee/cells/archive/attach-race-settle-proof/attach-race-settle-proof-2.json]
---

# attach-race-settle-proof — Delivery

## What shipped

- **attach-race-settle-proof-1** (commit 9abc20e) — exact 4-read assertion with a panic-on-5th-read mock, an elapsed bound proving stop-on-repeat (not the cap) ends the wait, a first-read timestamp assertion pinning the min-quiet window, an elapsed cap bound in the never-settles test, and test durations widened 5/5/40ms to 25/25/200ms. Test module only.
- **attach-race-settle-proof-2** (commit 9a503b4) — min-gap (≥80% of poll interval) and read-count-ceiling assertions pinning the poll's pacing, so a zeroed poll sleep fails on both the gap and the storm. Test module only.

## Verify

Both cells capped against `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green, each with a scratchpad-copy mutation proof quoted in its cap (mechanism deleted → named failing assertion).

## Provenance

Fix lane for the three P1s of review-2026-08-16-terminal-attach-submit-race (mutation-survivable settle proofs). The lesson generalized in `docs/history/learnings/20260816-a-lower-bound-assertion-proves-nothing.md`; no area bullets — client-invisible test hardening.
