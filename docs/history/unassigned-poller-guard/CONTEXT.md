# Unassigned Poller Guard — Context (small brief)

**Feature slug:** unassigned-poller-guard
**Date:** 2026-08-16
**Lane:** small · bugfix

## What was asked

Fix the backlog bug: the homepage terminal poller in `app.js` fires on the
Unassigned agents page with `projectId=null`, hitting `/p/null/_terminal/<pane>/…`
every 1.5s (404 flicker on healthy panes) and double-POSTing on Send/key.

## What was found (verified on HEAD)

- `app.js:903-905` — the screen poller resolves `projectId` from
  `main.fg-page[data-project-id]`; the Unassigned page's `<main class="fg-page">`
  (`views.rs:1771`) has **no** `data-project-id`, so `projectId=null`.
- `app.js:905` — the selector `.term-screen[data-pane-id]` is unscoped, matching
  the Unassigned panes; those panes carry **no** `data-term-base` (`views.rs:1237`,
  `pane_cards(..., false)`), so `validTermBase(...)` returns `null`.
- `screenUrl` (`app.js:932-935`) then builds `/p/null/…` because both `base` and
  `projectId` are null. Same for `inputUrl`/`keysUrl`/`attachUrl` (`app.js:1476-1497`)
  → double-POST on Send/key.
- The Unassigned page ALSO runs its own inline `UNASSIGNED_TERMINAL_SCRIPT`
  (`views.rs:1621-1754`), correctly scoped to `.unassigned-panes` — so the page
  works, but the shared poller spams `/p/null` in parallel. Not masked by the D3
  `validTermBase` fix.

## Locked Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D1 | The `app.js` global screen poller and its input/keys/attach posters skip any element that resolves to NEITHER a valid same-origin `data-term-base` NOR a non-null page `projectId` — a per-element bail-out before any fetch/post. | Stops `/p/null/…` polling and double-POST on any page without `data-project-id` (the Unassigned page today), while the page's own scoped inline poller keeps working. Decision log a16dfee3. |

Coordination: D1 resolves the open `/p/null` defect that `homepage-terminal-full`'s
plan records as its D9 (decision daff25da) — that plan's D9 should later drop or reduce to a check.

### Agent's Discretion

- Whether the bail-out is a shared helper reused across the screen poller and the
  three posters, or an inline check at each — prefer one helper if natural.
- Exact placement (inside `pollOne`/`sendComposed` vs at selector time).

## Existing Code Context

- `crates/waggledance/assets/app.js` — screen poller IIFE (~894-1114), `screenUrl`
  (932), `validTermBase` (15-29), posters `inputUrl`/`keysUrl`/`attachUrl` (~1476-1497).
- `crates/waggledance/src/views.rs` — Unassigned page `unassigned_terminal_page`
  (~1762), `pane_cards` (~1237). Referenced for the test that pins the markup contract.

## Verify

The predicate is JS with no harness. Proof is a Rust boundary test pinning the
markup contract the guard relies on: the Unassigned page's `<main>` has no
`data-project-id` and its `.term-screen` elements have no `data-term-base` — so a
correct guard MUST skip them. Plus the JS-only guard recorded as a named manual
browser check (the way `home-terminal-header-2` did): on `/_terminal/unassigned`,
no `/p/null/…` request fires and Send posts once.

## Handoff Note

One cell, one writer, one worktree. CONTEXT is the source of truth; D1 is stable.
