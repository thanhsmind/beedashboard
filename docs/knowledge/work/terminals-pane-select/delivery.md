---
type: bee.delivery
title: terminals-pane-select — delivery
description: "Delivery record for work item terminals-pane-select: the Terminals tab's pane switcher became a single select."
timestamp: 2026-08-14
bee:
  id: terminals-pane-select-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: []
  sources: [.bee/cells/terminals-pane-select-1.json]
---

# terminals-pane-select — Delivery

## What shipped

The Terminals tab's pane switcher is one dropdown instead of a row of buttons.
Each entry still reads the same way — project, status, program, and the pane
title when it has one — and the pane currently shown is the one preselected.
Choosing an entry moves to that pane.

Before this, one button per pane wrapped onto four lines on a phone and pushed
the terminal screen itself below the fold.

- **terminals-pane-select-1** — Switcher rendered as a select with its own
  styling; the pane strip on a project's own terminal page is a different
  control and is unchanged. 3 files changed.

## Behaviour that settled

- **Choosing moves you.** There is no confirm button; picking an entry navigates
  straight to that pane. This makes the switcher depend on scripting, where the
  buttons it replaces were plain links — a deliberate trade for the vertical
  space.
- **Everything else about the tab holds.** No pane named means the first pane;
  naming a pane that has gone still shows the "this terminal is gone" line with
  the full list still offered; the two empty states — nothing running, and the
  agent host unreachable — read as they did.

## Verify

- **terminals-pane-select-1** — `cargo test --workspace`, 944 passing.

## Deviations

- The worker was not registered in the session's worker list at cap time and
  registered itself before finishing. Bookkeeping only; no decision changed.
