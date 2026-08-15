---
type: bee.delivery
title: home-terminal-parity — delivery
description: "Delivery record for work item home-terminal-parity: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: home-terminal-parity-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: [docs/specs/agent-terminal.md, docs/specs/web-interface.md]
  sources: [.bee/cells/archive/home-terminal-parity/home-terminal-parity-1.json, .bee/cells/archive/home-terminal-parity/home-terminal-parity-2.json]
---

# home-terminal-parity — Delivery

## What shipped

The home page's terminal section was a reduced version of the real thing: a
live screen you could type into, chosen from a plain drop-down, with no way to
look back at what had scrolled past and no sign of which terminal you were
looking at beyond the drop-down's own wording.

It now carries the same controls the project's own terminal page carries.

- **Looking back.** The round column of older / newer / live buttons sits
  beside the screen, and history scrolling asks for the selected terminal's
  own address. That last part was the reason it could not simply be dropped
  in: the buttons resolved their address from a project identifier the home
  page does not have, while the live view had already learned to read each
  screen's own address. The buttons now read the same thing.
- **Knowing what you are watching.** A line above the screen names the
  terminal's project, its status, the program running in it and its title.
- **Switching.** The drop-down is gone. The same Agents drawer the project
  page uses opens from the edge of the page, but on the home page its rows
  group under project names rather than under statuses, and each row keeps
  you on the home page — it changes which terminal the tab is showing rather
  than sending you to that project's own page.
- **Starting one.** The new-shell and preset agent buttons are there too.
  They start the new terminal in the project the currently selected terminal
  belongs to. A terminal belonging to no project shows no such buttons,
  because starting one somewhere is only possible where a somewhere exists.

What deliberately did not come across: the project's own pane strip, its
Overview and Transcript navigation, and its breadcrumb. Those name one
project, and the home page's terminal tab spans all of them.

## Verify

`cargo test --workspace` green at 964, up from 959. New cases cover the
scroll stack on the home tab, the identity line's contents, the drawer
rendering in home-page mode with no drop-down left behind, create buttons
present for a project terminal and absent for one outside every project, and
the project page's own drawer grouping and links staying as they were.

Confirmed against the running daemon: comparing the two pages' rendered
markup, everything that remains different is project navigation, and the
create box on the home page carries the selected terminal's own project.

## Deviations

None recorded.

## Provenance

Written from the capped traces of `home-terminal-parity-1` and `-2`. The
choices — drawer instead of drop-down, grouping by project, and creating in
the selected terminal's project — are the user's, recorded in the decision
log. This work supersedes the drop-down that
[terminals-pane-select](../terminals-pane-select/delivery.md) introduced.
