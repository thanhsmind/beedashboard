---
type: bee.delivery
title: homepage-terminals — delivery
description: "Delivery record for work item homepage-terminals: 2 capped cell(s), 2 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: homepage-terminals-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [docs/specs/bee-cockpit.md, docs/specs/web-interface.md]
  sources: [.bee/cells/archive/homepage-terminals/homepage-terminals-1.json, .bee/cells/archive/homepage-terminals/homepage-terminals-2.json]
---

# homepage-terminals — Delivery

## What shipped

Watching an agent work used to mean picking its project first, then opening
that project's terminal section. An agent running outside every registered
project could only be reached through a separate page of its own. The home page
knew which agents were alive — it drew a badge for each one — but the badge led
nowhere.

The home page now carries a third tab, Terminals, beside Kanban and Projects.
It opens one agent's terminal, live, with a switch strip above it listing every
agent across every project plus the ones belonging to no project at all. Shell
sessions with no agent in them are left out: the tab is for watching agents, and
an idle shell is noise.

The strip is ordered by what needs a person: an agent blocked and waiting for an
answer comes first, then agents working, then the rest. That same order decides
which terminal opens when the address names none.

Each entry in the strip is a real link carrying the chosen terminal in the
address, so a reload, a bookmark, or the back button all land on the same
terminal rather than a default one. An address naming a terminal that has since
closed says so plainly and shows the full strip — it never quietly switches to a
different terminal, because the person may be about to type.

Typing works exactly as it does on a project's own terminal page: a reply box,
the named keys, and — for an agent inside a registered project — pasting an
image. Agents outside every project take text and keys but no image, matching
what that route has always offered.

Two empty states read differently on purpose. When the terminal service is not
running, the tab says so. When it is running and simply no agent is alive, the
tab says that instead. The tab itself never disappears, so its position on the
strip stays predictable.

Nothing new was opened to the network. Every read and every keystroke travels
the addresses the project terminal and the out-of-project terminal already
served, each keeping the check it already performed on which terminals a caller
may touch. The page hands each terminal its own address prefix and the browser
side follows it, which is what let a page holding terminals from several
projects at once reuse machinery built for one project at a time.

## Open gap

The tab strip is still suppressed entirely when no registered project carries
bee metadata — behaviour that predates this work. While that holds, the
Terminals tab cannot be reached even with agents running. Filed for a later
decision.

## Verify

`cargo test --workspace` green at 935, up from 928. Cases go through the
router: the tab serving its own section, shell sessions excluded while
out-of-project agents are included, the blocked-before-working order, an address
naming a closed terminal, the two empty states told apart, the reply box and
keys present, the image control offered only where the route supports it, and
each terminal's address prefix pointing at the route that owns it.

Not yet confirmed against the running daemon.

## Deviations

The work ran in the main checkout rather than its own branch checkout: this
session could not open one, and the branch checkout it had created held no work
yet, so it was removed and the files were held by reservation instead. Recorded
in the decision log.

The first cell's worker registration was written to the shared record of an
unrelated feature and removed again afterwards, which left the finished cell
looking unowned. It was capped with that reason recorded rather than by
pretending it had run inline.

## Provenance

Written at feature close from the capped cell traces of `homepage-terminals-1`
and `homepage-terminals-2`. The eight product decisions behind the tab — what it
lists, how it is ordered, what the address carries, and what the two empty
states say — are recorded in the decision log and in the feature's context
record.
