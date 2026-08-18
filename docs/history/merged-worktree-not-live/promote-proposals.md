promote proposal for work item "merged-worktree-not-live" (.bee/lanes/merged-worktree-not-live.json) — 2 capped cell(s): merged-worktree-not-live-1, merged-worktree-not-live-2
anchor: ledger — .bee/lanes/merged-worktree-not-live.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/merged-worktree-not-live/delivery.md

---
type: bee.delivery
title: merged-worktree-not-live — delivery
description: "Delivery record proposed by bee knowledge promote for work item merged-worktree-not-live: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-18
bee:
  id: merged-worktree-not-live-delivery
  lifecycle: active
  required_context: [.bee/lanes/merged-worktree-not-live.json]
  sources: [.bee/lanes/merged-worktree-not-live.json, .bee/cells/merged-worktree-not-live-1.json, .bee/cells/merged-worktree-not-live-2.json]
---

# merged-worktree-not-live — Delivery

## What shipped

- **merged-worktree-not-live-1** — BeeWorktree.merged_pending derived from deferred-queue.jsonl excludes merged-but-kept worktrees from feature-hub In Progress placement (3 file(s) changed)
- **merged-worktree-not-live-2** — Widened is_finished to bee's terminal phase set {idle, compounding-complete}; added idle-phase tests in views.rs and server.rs; existing compounding-complete tests unchanged. (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **merged-worktree-not-live-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- **merged-worktree-not-live-2** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work merged-worktree-not-live` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/lanes/merged-worktree-not-live.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.