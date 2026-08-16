promote proposal for work item "attach-race-settle-proof" (.bee/logs/scribing-runs.jsonl + .bee/lanes/attach-race-settle-proof.json) — 2 capped cell(s): attach-race-settle-proof-1, attach-race-settle-proof-2
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/attach-race-settle-proof.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/attach-race-settle-proof/delivery.md

---
type: bee.delivery
title: attach-race-settle-proof — delivery
description: "Delivery record proposed by bee knowledge promote for work item attach-race-settle-proof: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: attach-race-settle-proof-delivery
  lifecycle: active
  areas: [none]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/attach-race-settle-proof.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/attach-race-settle-proof.json, .bee/cells/attach-race-settle-proof-1.json, .bee/cells/attach-race-settle-proof-2.json]
---

# attach-race-settle-proof — Delivery

## What shipped

- **attach-race-settle-proof-1** — Pin both settle mechanisms so their removal turns the suite red (1 file(s) changed)
- **attach-race-settle-proof-2** — Pin the poll interval so a zeroed sleep turns the suite red (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **attach-race-settle-proof-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace green. Then the mutation proof, run against a SCRATCHPAD COPY of the repo, never the working tree: (a) with the identical-text early return deleted, the settle tests go red; (b) with the min-quiet sleep deleted, the settle tests go red. Quote the failing test name and assertion from each red run in the cap message, then confirm git status --porcelain is clean in the real tree.`
- **attach-race-settle-proof-2** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace green. Mutation proof against a SCRATCHPAD COPY only, never the worktree: with the poll sleep at socket.rs:313 deleted (or poll_interval forced to zero), the settle tests go red; quote the failing test name and assertion. Confirm git status --porcelain shows only the intended test-module change.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work attach-race-settle-proof` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/attach-race-settle-proof.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "attach-race-settle-proof" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-16T04:44:58.947Z), the work item declares no bee.areas.

area none:
  (no capped behavior_change cell exists for this feature)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.