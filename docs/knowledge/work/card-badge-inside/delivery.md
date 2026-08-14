---
type: bee.delivery
title: card-badge-inside — delivery
description: "Delivery record for work item card-badge-inside: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: card-badge-inside-delivery
  lifecycle: active
  areas: [bee-cockpit, appearance]
  required_context: [docs/specs/bee-cockpit.md, docs/specs/appearance.md]
  sources: [.bee/cells/archive/card-badge-inside/card-badge-inside-1.json]
---

# card-badge-inside — Delivery

## What shipped

On the Features board, the markers naming the terminal sessions running in a
feature's checkout now sit inside that feature's card, at its foot, separated
from the card's own lines by a hairline rule. Before this they hung below the
card as a loose row on the page background, reading as though they belonged to
the gap between two cards rather than to either one.

The card's own text is a single link to the feature, and a link cannot contain
another link — which is why the session markers, each a link to its terminal,
could never be placed inside it. The card and its markers are now drawn inside
a shared container, and that container is what carries the card's frame: the
border, the background, the rounded corner and the inner spacing. The link
itself keeps every line it had and stays the whole reading surface; each marker
stays separately clickable. Neither link nests inside the other.

A feature with no session running in its checkout renders exactly as before —
no rule, no empty strip, nothing to say there is nothing to show.

The same markers on the project list are untouched: they keep the spacing that
their own row layout needs, so only the board's cards changed.

## Verify

`cargo test --workspace` green at 909. The existing shape test still pins that
the marker group follows the card link rather than nesting inside it, and gained
assertions that both are wrapped by the frame-carrying container, that the link
no longer carries the frame itself, and that the group closes before the
container does. A case covers a card with no sessions rendering the container
and the link but no marker group. The project-list markers are asserted
byte-identical to before.

Confirmed against the running daemon: the live board serves five cards, three of
them carrying their marker group inside the container.

## Deviations

The work ran in the main checkout rather than its own feature branch checkout.
The branch checkout was created first, but this session cannot write outside the
main one, so no worker could reach the files there. The change is one cell
touching two files with no other session holding them, which is the recorded
exception for work this small. The stray checkout and its branch were removed.

## Provenance

Written at feature close from the capped cell trace of `card-badge-inside-1`.
The arrangement itself — a shared frame-carrying container, markers at the foot
behind a rule — was chosen by the owner from three sketched options and is
recorded in the decision log.
