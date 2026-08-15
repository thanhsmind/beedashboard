---
type: bee.delivery
title: home-terminal-header — delivery
description: "Delivery record for work item home-terminal-header: 2 capped cell(s), 2 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: home-terminal-header-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: [docs/specs/agent-terminal.md, docs/specs/web-interface.md]
  sources: [.bee/cells/home-terminal-header-1.json, .bee/cells/home-terminal-header-2.json]
---

# home-terminal-header — Delivery

## What shipped

The home page's terminal section named the terminal it was showing in a
single line of small grey text, set in the same tone as every other piece of
chrome around it. Someone scanning the tab could not tell at a glance which
of several terminals they were watching. It also offered a plain new-shell
button, which could only ever start a shell in whichever project the
terminal on screen happened to belong to.

- **Knowing what you are watching.** The terminal now names itself in a
  header above its own screen. The first line carries its status, its project
  and its window-and-tab name, at the weight of a heading. The second keeps
  the program and title in the quieter tone underneath. A rule closes the
  block, so the header stops where the screen starts. The window-and-tab pair
  is deliberately the same one the project page's own strip prints — a
  terminal reads the same on both surfaces.
- **Starting one.** The plain new-shell button is gone from the home page.
  Reaching and starting a terminal from there is the Agents drawer's job, and
  a shell button sitting above someone else's running screen only ever meant
  "a shell in that screen's project" — which is not what the home page is
  about. The preset agent buttons stay, still starting in the selected
  terminal's own project.
- **Nothing left to offer.** With the shell button gone, a setup that
  configures no agent presets has nothing to put in that box at all, so the
  box itself no longer renders. Same silence a terminal belonging to no
  project already gets, for the same reason.

The project's own terminal page keeps its new-shell button and its presets
exactly as they were. That is where "new shell here" has a project to mean.

- **One Agents drawer, not two.** The drawer is a switcher across every
  project, but it rearranged itself depending on which page you opened it
  from — headings by status on a project's terminal page, headings by
  project name on the home tab. What a reader is looking for in it is which
  project an agent belongs to, and the status shape buried that in the small
  print at the end of each row. Both pages now group under project names,
  with the rows inside each project ordered waiting first, then working,
  then the rest. The rows themselves are unchanged, and each page's rows
  still lead where they always did: the home tab keeps you on the home tab,
  the project page opens the agent's own page.

## Verify

`cargo test --workspace` green at 966, up from 964. New cases cover the
home page rendering no shell button while its presets still stand, and a
project terminal with no presets configured producing no create box at all.
The three project-page shell-button tests were left untouched on purpose —
they are the proof the button stayed where it belongs. Two existing home-page
tests were rewritten to read the header's two lines instead of the old single
line; every fact they asserted before is still asserted, only on a different
line.

The drawer change is browser code, and the suite is a Rust one that cannot
reach it — there is no JavaScript test harness in this repository, which is
the honest gap here. It was syntax-checked and read against its callers. The
daemon serves this file compiled into its binary and refuses to run twice at
once, so seeing the real drawer means installing this build; deferred to
after the merge, at the owner's call.

## Deviations

- Both cells ran inline rather than through a dispatched worker. The first
  dispatched worker came back blocked: its session was rooted in the main
  checkout, and the write guard refuses every write that lands inside a
  granted worktree from a session not physically rooted there. The
  orchestrator moved its own session into the worktree and ran the cells
  there. Recorded on each cap's own trace.
- The first cell was routed without a route record. The route command writes
  to whichever feature the shared control plane holds active, and that was
  another session's live feature at the time. The fix was to bind this
  session to its own lane, after which the route recorded normally — the
  second cell carries one.

## Provenance

Written from the capped traces of `home-terminal-header-1` and `-2`. Every
choice here — a header rather than a caption, dropping the shell button while
keeping the presets, and unifying the drawer on project grouping with the row
left alone — is the user's, taken in the shaping interview. This work amends
the identity line and create controls that
[home-terminal-parity](../home-terminal-parity/delivery.md) introduced.
