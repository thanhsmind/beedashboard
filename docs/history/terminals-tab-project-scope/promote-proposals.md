promote proposal for work item "terminals-tab-project-scope" (.bee/lanes/terminals-tab-project-scope.json + docs/history/terminals-tab-project-scope/promote-proposals.md) — 1 capped cell(s): terminals-tab-project-scope-1
anchor: ledger — .bee/lanes/terminals-tab-project-scope.json, docs/history/terminals-tab-project-scope/promote-proposals.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/terminals-tab-project-scope/delivery.md

---
type: bee.delivery
title: terminals-tab-project-scope — delivery
description: "Delivery record proposed by bee knowledge promote for work item terminals-tab-project-scope: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: terminals-tab-project-scope-delivery
  lifecycle: active
  required_context: [.bee/lanes/terminals-tab-project-scope.json, docs/history/terminals-tab-project-scope/promote-proposals.md]
  sources: [.bee/lanes/terminals-tab-project-scope.json, docs/history/terminals-tab-project-scope/promote-proposals.md, .bee/cells/terminals-tab-project-scope-1.json]
---

# terminals-tab-project-scope — Delivery

## What shipped

- **terminals-tab-project-scope-1** — Scope the Terminals tab switcher to the active pane's project (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **terminals-tab-project-scope-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work terminals-tab-project-scope` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/lanes/terminals-tab-project-scope.json`, `docs/history/terminals-tab-project-scope/promote-proposals.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.