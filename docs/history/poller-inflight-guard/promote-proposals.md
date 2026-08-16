promote proposal for work item "poller-inflight-guard" (.bee/lanes/poller-inflight-guard.json) — 1 capped cell(s): poller-inflight-guard-1
anchor: ledger — .bee/lanes/poller-inflight-guard.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/poller-inflight-guard/delivery.md

---
type: bee.delivery
title: poller-inflight-guard — delivery
description: "Delivery record proposed by bee knowledge promote for work item poller-inflight-guard: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: poller-inflight-guard-delivery
  lifecycle: active
  required_context: [.bee/lanes/poller-inflight-guard.json]
  sources: [.bee/lanes/poller-inflight-guard.json, .bee/cells/poller-inflight-guard-1.json]
---

# poller-inflight-guard — Delivery

## What shipped

- **poller-inflight-guard-1** — Added inFlightScreen[paneId] guard to the screen poller's pollOne (crates/waggledance/assets/app.js), mirroring the transcript poller's inFlight pattern: skip a tick if the pane's fetch is still outstanding, set before fetch, clear on both success and error settle paths. Interval, URL building, hasTarget/validTermBase bail-out, and transcript poller untouched. cargo test --workspace: 1025 passed green. JS-only guard has no repo harness (pure client timing) -- recorded per home-terminal-header-2 precedent: manual browser check on a project terminal page with a slow/hung pane confirms only one screen fetch per pane outstanding at a time. (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **poller-inflight-guard-1** — `cargo test --workspace green (unchanged Rust suite — the guard is client JS). Record the JS-only guard the way home-terminal-header-2 did: manual browser check that only one screen fetch per pane is outstanding at a time under a slow/hung pane. State it in the commit body and cap outcome.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work poller-inflight-guard` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/lanes/poller-inflight-guard.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.