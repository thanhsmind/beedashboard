---
type: bee.delivery
title: doctor-preserve-order — delivery
description: "Delivery record for work item doctor-preserve-order: one capped cell stopping the health-check fixer from reordering a user's configuration file."
timestamp: 2026-08-20
bee:
  id: doctor-preserve-order-delivery
  lifecycle: active
  areas: [doctor]
  required_context: [docs/history/doctor-preserve-order/promote-proposals.md]
  sources: [docs/history/doctor-preserve-order/promote-proposals.md]
---

# doctor-preserve-order — Delivery

## What shipped

- **dpo-1** — The health-check fixer now leaves a user's configuration file in the order they wrote it: registering the MCP server changes only its own entry, and every unrelated key keeps its position. Previously each fix silently rewrote the whole file in alphabetical order — content survived, ordering and grouping did not.

## How it was verified

- `cargo test -p waggledance-core notify_store` is unrelated; this feature's proof is `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` — green, 1141 passed at the time of the cap.
- Red-first: with the ordering guarantee removed, the new test fails and the rewritten file comes back alphabetised. That failure was observed before the fix was accepted.

## Why it stays fixed

The guarantee is structural, not a convention someone must remember: the serialization layer is configured to preserve insertion order, so a future writer cannot reintroduce the reordering by forgetting a rule. One test pins it by seeding deliberately non-alphabetical keys and asserting their relative order survives a fix.

## Where the record lives

The cell record for dpo-1 is bee bookkeeping, not a durable source: its path moves between the hot store and the archive every time the feature is archived or reopened, so citing it here only rots. The durable trail is the commit `ac7ef14` and the promote proposals beside it.

## Open gap

Nothing here covers the other configuration formats the fixer touches; only the JSON path is pinned.
