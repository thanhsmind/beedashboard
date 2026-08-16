promote proposal for work item "hub-card-title-size" (.bee/lanes/hub-card-title-size.json) — 1 capped cell(s): hub-card-title-size-1
anchor: ledger — .bee/lanes/hub-card-title-size.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/hub-card-title-size/delivery.md

---
type: bee.delivery
title: hub-card-title-size — delivery
description: "Delivery record proposed by bee knowledge promote for work item hub-card-title-size: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: hub-card-title-size-delivery
  lifecycle: active
  required_context: [.bee/lanes/hub-card-title-size.json]
  sources: [.bee/lanes/hub-card-title-size.json, .bee/cells/hub-card-title-size-1.json]
---

# hub-card-title-size — Delivery

## What shipped

- **hub-card-title-size-1** — Match hub card title size to column subhead (0 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **hub-card-title-size-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work hub-card-title-size` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/lanes/hub-card-title-size.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.