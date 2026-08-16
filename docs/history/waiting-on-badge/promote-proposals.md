promote proposal for work item "waiting-on-badge" (.bee/lanes/waiting-on-badge.json) — 1 capped cell(s): wob-1
anchor: ledger — .bee/lanes/waiting-on-badge.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/waiting-on-badge/delivery.md

---
type: bee.delivery
title: waiting-on-badge — delivery
description: "Delivery record proposed by bee knowledge promote for work item waiting-on-badge: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: waiting-on-badge-delivery
  lifecycle: active
  required_context: [.bee/lanes/waiting-on-badge.json]
  sources: [.bee/lanes/waiting-on-badge.json, .bee/cells/wob-1.json]
---

# waiting-on-badge — Delivery

## What shipped

- **wob-1** — Gate the Awaiting approval badge on a live waiting_on mark (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **wob-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work waiting-on-badge` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/lanes/waiting-on-badge.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.