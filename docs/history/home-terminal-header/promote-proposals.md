promote proposal for work item "home-terminal-header" (.bee/logs/scribing-runs.jsonl + .bee/lanes/home-terminal-header.json) — 2 capped cell(s): home-terminal-header-1, home-terminal-header-2
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/home-terminal-header.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/home-terminal-header/delivery.md

---
type: bee.delivery
title: home-terminal-header — delivery
description: "Delivery record proposed by bee knowledge promote for work item home-terminal-header: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: home-terminal-header-delivery
  lifecycle: active
  areas: [agent-terminal, web-interface]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/home-terminal-header.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/home-terminal-header.json, .bee/cells/home-terminal-header-1.json, .bee/cells/home-terminal-header-2.json]
---

# home-terminal-header — Delivery

## What shipped

- **home-terminal-header-1** — done (2 file(s) changed)
- **home-terminal-header-2** — done (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **home-terminal-header-1** — `cargo test --workspace green. The existing server.rs homepage-tab create-controls test flips to assert the tab renders no New shell button while a configured preset button still renders and .term-create still targets the selected pane's own project. A new case asserts the homepage tab's header block renders both lines - project label plus pane identity on the title line, program and title on the sub line. The three project-page New shell tests (views.rs terminal_page_lists_only_configured_preset_labels, terminal_page_puts_the_section_nav_in_the_bar_and_new_shell_on_the_pane_row, the_pane_bar_grows_no_menu_when_there_are_no_panes) stay untouched and green, proving the button stayed where it belongs. An edge case asserts a homepage-selected project pane with zero configured presets renders no .term-create box at all.`
- **home-terminal-header-2** — `cargo test --workspace green as a regression check only - it is a Rust suite and cannot reach this file; the repo carries no JS test harness, which is the named gap for this cell. Real verification is a browser pass against the running daemon: open a project terminal page and the homepage Terminals tab, open the Agents drawer on each, and confirm both group under project-name headings with the same row shape and the same blocked-before-working order, and that each instance's rows still link where they did before.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work home-terminal-header` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/home-terminal-header.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "home-terminal-header" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-15T15:47:56.864Z), the work item declares no bee.areas.

area agent-terminal:
  - [home-terminal-header-1] done — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/home-terminal-header-1.json)
  - [home-terminal-header-2] done — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/home-terminal-header-2.json)

area web-interface:
  - [home-terminal-header-1] done — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/home-terminal-header-1.json)
  - [home-terminal-header-2] done — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/home-terminal-header-2.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 4 area bullet(s), 0 pattern candidate(s), 0 file(s) written.