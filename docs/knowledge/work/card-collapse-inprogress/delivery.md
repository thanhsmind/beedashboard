---
type: bee.delivery
title: card-collapse-inprogress — delivery
description: "Delivery record for work item card-collapse-inprogress: the In Progress card now ships collapsed and opens on a click."
timestamp: 2026-08-15
bee:
  id: card-collapse-inprogress-delivery
  lifecycle: active
  areas: [web-interface]
  required_context: [docs/specs/web-interface.md]
  sources: [docs/history/card-collapse-inprogress/CONTEXT.md, .bee/cells/archive/card-collapse-inprogress/card-collapse-inprogress-1.json, .bee/cells/archive/card-collapse-inprogress/card-collapse-inprogress-2.json]
---

# card-collapse-inprogress — Delivery

## What shipped

The In Progress column was the one column that still drew full cards, and
each card drew everything it knew at once — its name, the project and
worktree it belongs to, a description, a progress bar, a last-activity line.
With several features running the column became a wall of text to scroll
past.

A card now arrives closed. It shows its name and a chevron, and nothing
else — except its terminal badges, which stay at the foot of the card
whether it is open or closed, because a running terminal is the one thing
worth seeing without opening anything.

- **Opening.** Clicking anywhere on the card's header row opens it; clicking
  again closes it. The whole header is the target, which is what makes it
  usable on a phone.
- **Reaching the feature.** The card used to be one big link. It cannot be
  both a link and a toggle, so the link moved: an open card's body starts
  with its own `Feature detail` row carrying an arrow, and that row is what
  goes to the feature's page.
- **What the open card says.** Exactly what the card said before — the
  project and worktree line, the description clamped to two lines, the
  progress bar with its `N/M cells done` label, the reason line, the
  last-activity line — in the same order and the same shapes. The reference
  the user brought showed data as `label ······ value` rows; that was
  deliberately not adopted, because a progress bar reads faster than a
  sentence.
- **Where.** Both boards: the cross-project board on the home page and each
  project's own feature board. One renderer draws both, so they cannot
  drift.

The open/closed state is not remembered. Every page load draws every card
closed. Nothing about the toggle involves scripting or browser storage — the
disclosure is native, so keyboard access and the open/closed state announced
to a screen reader come for free.

## Verify

`cargo test --workspace` green at 966, up from 964. Cases cover a card
rendering with no open attribute (the whole point — a regression that
shipped cards pre-expanded would otherwise pass silently), the body opening
with a detail link carrying the same address the old whole-card link
carried, the badge navigation sitting outside the disclosure so a closed
card still shows it, and the empty-pane case still drawing no badge
container at all.

Confirmed against the running daemon: the served markup carries the new
header and detail-link classes.

## Deviations

The two collapse-specific test cases were missing from the first cell and
were caught by a goal-check, not by the test run — a card can ship
pre-expanded and every existing assertion still passes. A second cell added
them.

## Provenance

Written from the capped traces of `card-collapse-inprogress-1` and `-2` and
the locked decisions in `docs/history/card-collapse-inprogress/CONTEXT.md`.
The card this work reshaped is the one
[project-color-identity](../project-color-identity/delivery.md) gave its
accent border and project subtitle, and
[card-badge-inside](../card-badge-inside/index.md) moved the badges into.
[inprogress-priority-order](../inprogress-priority-order/delivery.md) then
decided what order these cards appear in.
