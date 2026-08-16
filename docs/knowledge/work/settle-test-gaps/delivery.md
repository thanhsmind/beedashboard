---
type: bee.delivery
title: settle-test-gaps — delivery
description: "Delivery record for work item settle-test-gaps: the settle wait's untested branches covered and the trait doc's dead-revision claim corrected."
timestamp: 2026-08-16
bee:
  id: settle-test-gaps-delivery
  lifecycle: active
  required_context: [docs/knowledge/work/terminal-attach-submit-race/delivery.md]
  sources: [.bee/lanes/settle-test-gaps.json, .bee/cells/archive/settle-test-gaps/settle-test-gaps-1.json]
---

# settle-test-gaps — Delivery

## What shipped

- **settle-test-gaps-1** (commit 4ee0e6e) — three new tests: read-error fall-through (exactly one pane.read, then the Enter, Ok result), submit=false (exactly one text request, zero reads), empty-text submit (exactly one enter request, zero reads); the two-requests test pinned from a presence check to exactly 2 reads; between-read request shape asserted (pane_id, source visible, no lines key); and the Herdr::send_input trait doc corrected to state the settle wait compares screen TEXT, not the dead revision field. Test module + one doc block; no production code.

## Verify

Committed path-scoped while a sibling's WIP held the workspace red (recorded fix-first); capped after the tree cleared against the fresh full gate: fmt clean, clippy clean, `cargo test --workspace` 1047 passed.

## Provenance

Batch of the P2/P3 test-coverage remedies from review-2026-08-16-terminal-attach-submit-race that were disjoint from live sibling work. No area bullets — client-invisible coverage.
