---
type: bee.delivery
title: unassigned-poller-guard — delivery
description: "Delivery record for work item unassigned-poller-guard: a page never polls or posts to a pane it cannot address — no more /p/null requests from the Unassigned page."
timestamp: 2026-08-16
bee:
  id: unassigned-poller-guard-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [docs/specs/agent-terminal.md]
  sources: [.bee/lanes/unassigned-poller-guard.json, .bee/cells/unassigned-poller-guard-1.json]
---

# unassigned-poller-guard — Delivery

## What shipped

- **unassigned-poller-guard-1** — Added a shared hasTarget(base, projectId) helper in app.js and used it as a per-element bail-out in the screen poller (pollOne) and all three poster loops (forms, keyGroups, and the scroll Older/Newer/Live group — covering input/keys/attach) so no element without a valid data-term-base or a page projectId ever fetches/posts /p/null/.... Rust boundary test in views.rs pins the markup contract (Unassigned page's `<main>` has no data-project-id and its panes carry no data-term-base; the project page's `<main>` carries data-project-id; the homepage Terminals tab's pane carries data-term-base). cargo test --workspace: 1023 passed. JS guard itself has no repo harness — manual browser check recorded in the test doc comment: on /_terminal/unassigned no /p/null request fires across several poll ticks and Send posts once. (2 files changed: app.js, views.rs)

## Verify

Capped against `cargo test --workspace` green, with the views.rs boundary test pinning the markup contract the guard relies on, and the JS-only guard recorded with its manual browser check per the home-terminal-header-2 precedent.

## Deviations

None recorded in the capped cell trace.

## Provenance

Proposed by `bee knowledge promote --work unassigned-poller-guard` from 1 capped cell trace. Accepted at the compounding pass on 2026-08-16 and saved here as the factual delivery record. The proposal's one agent-terminal area bullet was checked against the living spec and found already merged by the feature's own scribing sync: `docs/specs/agent-terminal.md` states "Which panes a page keeps polling and driving: only the ones it can address" (commit 1909d65). No pattern candidates were proposed.
