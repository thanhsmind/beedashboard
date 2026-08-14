---
type: bee.delivery
title: pbi-detail — delivery
description: "Delivery record for work item pbi-detail: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: pbi-detail-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [docs/specs/bee-cockpit.md, docs/specs/web-interface.md]
  sources: [.bee/cells/archive/pbi-detail/pbi-detail-1.json]
---

# pbi-detail — Delivery

## What shipped

Every item on the feature board opens the thing it names — except the proposed
backlog items in the first column, which opened the whole board they were
already sitting on. They had nowhere else to go: a proposed item has no
started work behind it, and the page each other item opens is a page about
started work.

A proposed item now has a page of its own, reached by its own address. It
shows the item's title, its status, and the text that says what would count as
satisfying it. When the item names work the project already knows about, that
work is a link; when it names work that does not exist yet, the name is shown
without a link rather than a link that leads nowhere. A way back to the board
sits at the bottom.

An address naming no item answers the same way an address naming no started
work already did, rather than inventing a second kind of refusal.

Both boards agree: the project's own board and the cross-project board on the
home page build the link the same way, through one shared address builder.

## Verify

`cargo test --workspace` green at 947, up from 944. Three tests that asserted
the old destination were updated, and three added: the page with a linked
piece of work, the page where that work does not exist and is therefore not
linked, and an unknown address.

Confirmed against the running daemon: every proposed item on the home board
links to its own address, that page renders the item's status and satisfaction
text, and an unknown item answers 404.

## Deviations

None recorded. The dispatched worker stalled once mid-read and was restarted
with narrower instructions; no work had been written at that point.

## Provenance

Written from the capped cell trace of `pbi-detail-1`. The choice to build a
per-item page rather than scrolling the board to the item is recorded in the
decision log.
