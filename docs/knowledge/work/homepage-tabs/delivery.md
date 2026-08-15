---
type: bee.delivery
title: homepage-tabs — delivery
description: "Delivery record for work item homepage-tabs: 1 capped cell(s), 1 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: homepage-tabs-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [docs/specs/bee-cockpit.md, docs/specs/web-interface.md]
  sources: [.bee/cells/archive/homepage-tabs/homepage-tabs-1.json]
---

# homepage-tabs — Delivery

## What shipped

The home page used to stack two full sections on top of each other: the board
of features across every project, and beneath it the list of projects with its
registration form. Reaching the list meant scrolling past however many feature
cards the board happened to hold that day.

The two are now tabs. One strip at the top of the page offers Kanban and
Projects, and only the chosen one is built — the page never pays to render the
half nobody asked for.

Each tab is a real link with its own address, so the choice lives in the
address bar. Copying the address shares the tab it was on, the browser's back
button steps through tabs, and the choice survives the page reloading itself —
which it does on its own whenever a watched file changes, and which would
otherwise throw the reader back to the board mid-read. It works with scripting
turned off. An address naming a tab that does not exist opens the board.

Two cases keep their old behaviour on purpose. When nothing qualifies for the
board there is no strip at all, because a tab leading somewhere empty is worse
than no tab. And when registering a project fails, the Projects tab is shown
whatever the address asked for, since the failure message lives inside it and
would otherwise be invisible to the person who just submitted the form.

The strip reuses a tab component the design system had carried for a while
without a single caller, so no second tab style entered the codebase.

## Verify

`cargo test --workspace` green at 928, up from 924. Cases go through the router:
each tab serving its own section and not the other, the strip carrying exactly
one selected tab, an unknown or empty or repeated tab value falling back to the
board, a failed registration serving the Projects tab against an explicit
request for the board, and a home page with nothing on its board serving no
strip.

Confirmed against the running daemon: `/`, `/?tab=kanban`, `/?tab=projects` and
`/?tab=bogus` each serve the expected section with the expected tab marked.

## Deviations

The work ran in the main checkout rather than its own branch checkout, for the
reason recorded against `card-badge-inside`.

The worker's own registration was missing when it went to close its work, so it
registered itself as the tool instructed and retried.

## Provenance

Written at feature close from the capped cell trace of `homepage-tabs-1`. The
choice to put the tab in the address rather than in browser storage, and the
two cases that keep their old behaviour, are recorded in the decision log.
