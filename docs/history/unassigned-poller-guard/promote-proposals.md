promote proposal for work item "unassigned-poller-guard" (.bee/logs/scribing-runs.jsonl + .bee/lanes/unassigned-poller-guard.json) — 1 capped cell(s): unassigned-poller-guard-1
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/unassigned-poller-guard.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/unassigned-poller-guard/delivery.md

---
type: bee.delivery
title: unassigned-poller-guard — delivery
description: "Delivery record proposed by bee knowledge promote for work item unassigned-poller-guard: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: unassigned-poller-guard-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/unassigned-poller-guard.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/unassigned-poller-guard.json, .bee/cells/unassigned-poller-guard-1.json]
---

# unassigned-poller-guard — Delivery

## What shipped

- **unassigned-poller-guard-1** — Added a shared hasTarget(base, projectId) helper in app.js and used it as a per-element bail-out in the screen poller (pollOne) and the two posters loops (forms/keyGroups, covering input/keys/attach) so no element without a valid data-term-base or a page projectId ever fetches/posts /p/null/.... Rust boundary test in views.rs pins the markup contract (Unassigned page's <main> has no data-project-id and its panes carry no data-term-base; the project page's <main> carries data-project-id; the homepage Terminals tab's pane carries data-term-base). cargo test --workspace: 1023 passed. JS guard itself has no repo harness -- manual browser check recorded in the test doc comment: on /_terminal/unassigned no /p/null request fires across several poll ticks and Send posts once. (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **unassigned-poller-guard-1** — `cargo test --workspace green. A Rust boundary test in views.rs pins the markup contract the guard relies on: the Unassigned page (unassigned_terminal_page) renders <main class=fg-page> with NO data-project-id and its .term-screen panes carry NO data-term-base, while the project terminal page / homepage tab DO provide one of the two, so a guard that skips 'neither base nor projectId' targets exactly the Unassigned panes and nothing else. The JS guard itself has no repo harness: record it as a JS-only guard the way home-terminal-header-2 did, naming the manual browser check (on /_terminal/unassigned no /p/null request fires and Send posts once).`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work unassigned-poller-guard` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/unassigned-poller-guard.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "unassigned-poller-guard" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-16T02:13:31.945Z), the work item declares no bee.areas.

area agent-terminal:
  - [unassigned-poller-guard-1] Added a shared hasTarget(base, projectId) helper in app.js and used it as a per-element bail-out in the screen poller (pollOne) and the two posters loops (forms/keyGroups, covering input/keys/attach) so no element without a valid data-term-base or a page projectId ever fetches/posts /p/null/.... Rust boundary test in views.rs pins the markup contract (Unassigned page's <main> has no data-project-id and its panes carry no data-term-base; the project page's <main> carries data-project-id; the homepage Terminals tab's pane carries data-term-base). cargo test --workspace: 1023 passed. JS guard itself has no repo harness -- manual browser check recorded in the test doc comment: on /_terminal/unassigned no /p/null request fires across several poll ticks and Send posts once. — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/unassigned-poller-guard-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.