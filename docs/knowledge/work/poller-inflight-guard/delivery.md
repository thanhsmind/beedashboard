---
type: bee.delivery
title: poller-inflight-guard — delivery
description: "Delivery record for work item poller-inflight-guard: the screen poller never stacks a second fetch on a pane whose previous fetch is still outstanding."
timestamp: 2026-08-16
bee:
  id: poller-inflight-guard-delivery
  lifecycle: active
  required_context: [docs/specs/agent-terminal.md]
  sources: [.bee/lanes/poller-inflight-guard.json, .bee/cells/poller-inflight-guard-1.json]
---

# poller-inflight-guard — Delivery

## What shipped

- **poller-inflight-guard-1** — Added inFlightScreen[paneId] guard to the screen poller's pollOne (crates/waggledance/assets/app.js), mirroring the transcript poller's inFlight pattern: skip a tick if the pane's fetch is still outstanding, set before fetch, clear on both success and error settle paths. Interval, URL building, hasTarget/validTermBase bail-out, and transcript poller untouched. cargo test --workspace: 1025 passed green. JS-only guard has no repo harness (pure client timing) — recorded per home-terminal-header-2 precedent: manual browser check on a project terminal page with a slow/hung pane confirms only one screen fetch per pane outstanding at a time. (1 file(s) changed)

## Verify

Capped against `cargo test --workspace` green (unchanged Rust suite — the guard is client JS), with the JS-only guard recorded via its manual browser check per the home-terminal-header-2 precedent.

## Deviations

None recorded in the capped cell trace.

## Provenance

Proposed by `bee knowledge promote --work poller-inflight-guard` from 1 capped cell trace. Accepted at the compounding pass on 2026-08-16 and saved here as the factual delivery record. The proposal carried no area bullets, and the decision log records that nothing settled at the area-spec level: the in-flight guard is internal client-side fetch scheduling with no user-observable rule beyond what the spec already states. No pattern candidates were proposed.
