---
type: bee.delivery
title: hub-card-title-size — delivery
description: "Delivery record for work item hub-card-title-size: hub card titles render at the same size as their column headers."
timestamp: 2026-08-16
bee:
  id: hub-card-title-size-delivery
  lifecycle: active
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/lanes/hub-card-title-size.json, .bee/cells/hub-card-title-size-1.json]
---

# hub-card-title-size — Delivery

## What shipped

- **hub-card-title-size-1** — Match hub card title size to column subhead (CSS-only sizing change, commit c01272e).

## Verify

Capped against the full gate: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` green.

## Deviations

None recorded in the capped cell trace.

## Provenance

Proposed by `bee knowledge promote --work hub-card-title-size` from 1 capped cell trace. Accepted at the compounding pass on 2026-08-16. The work declares no areas and the change is a cosmetic sizing alignment with no behavior rule the specs track; no area bullet or pattern candidate was proposed and none was invented.
