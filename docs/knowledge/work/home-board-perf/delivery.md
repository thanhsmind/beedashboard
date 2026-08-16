---
type: bee.delivery
title: home-board-perf — delivery
description: "Delivery record for work item home-board-perf: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: home-board-perf-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/home-board-perf.json, .bee/cells/home-board-perf-1.json, .bee/cells/home-board-perf-2.json]
---

# home-board-perf — Delivery

## What shipped

- **home-board-perf-1** — Added a per-project Mutex<HashMap> cache on AppState keyed by a stat-only .bee/+docs/history fingerprint (max mtime, entry count); cross_project_rollup and bee_board now read through cached_read_rollup, read_snapshot/read_rollup stay pure. 4 new tests prove cache-hit Arc identity and invalidation on add/remove/edit. (1 file(s) changed)
- **home-board-perf-2** — Added isBoardRelevant predicate; home (!m) branch reloads only on docs/history-relevant changes. fmt/clippy/tests green (1056 passed). Manual browser check recorded per JS-only guard convention. (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **home-board-perf-1** — `cargo test --workspace green (CI triple fmt+clippy+test). New tests in server (or core): (1) two consecutive cached_snapshot calls on an unchanged fixture project return the same snapshot WITHOUT re-parsing — prove via a call counter, an Arc identity, or a spy that read_snapshot ran once; (2) touching/adding/removing a .bee file invalidates (second call re-reads); (3) changing a docs/history/<feature>/CONTEXT.md invalidates. Do not weaken any existing bee-snapshot test.`
- **home-board-perf-2** — `cargo test --workspace green (CI triple). The predicate is client JS with no repo harness: record the JS-only guard per home-terminal-header-2 (manual browser check: on / a changed list of only non-docs/history markdown does not reload, one containing a docs/history path does). If any Rust test asserts the old home-always-reloads behavior, none is expected — but if a served-HTML test references shouldReload, keep it green.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work home-board-perf` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/home-board-perf.json`. Applied 2026-08-16 from docs/history/home-board-perf/promote-proposals.md; the proposal's area bullets were deliberately not applied — disposition recorded in the decision log (backlog-groom-2/home-board-perf promote disposition).
