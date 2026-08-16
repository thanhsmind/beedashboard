---
type: bee.delivery
title: backlog-groom-2 — delivery
description: "Delivery record for work item backlog-groom-2: 5 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: backlog-groom-2-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/backlog-groom-2.json, .bee/cells/backlog-groom-2-1.json, .bee/cells/backlog-groom-2-2.json, .bee/cells/backlog-groom-2-3.json, .bee/cells/backlog-groom-2-4.json, .bee/cells/backlog-groom-2-5.json]
---

# backlog-groom-2 — Delivery

## What shipped

- **backlog-groom-2-1** — home_page no longer swallows the tab strip when the bee board is empty; Kanban shows its own fg-empty state, Projects keeps the project list, Terminals stays reachable; updated 20 pre-existing tests that assumed the old tabless early-return and added two new unit tests (2 file(s) changed)
- **backlog-groom-2-2** — Fell back to shell for an agentless pane's name in project_panes so every aria-label interpolation is non-empty; added terminal_pane_page_a_shell_rows_aria_labels_read_shell_not_empty covering shell + agent panes. (1 file(s) changed)
- **backlog-groom-2-3** — Added a fg-banner--warning strip under the cross-project board header, shown only when BeeSnapshot::read_errors is non-empty across the board's rollups; two new views.rs tests cover the with/without cases. cargo test --workspace green (1049 passed), fmt/clippy clean. (1 file(s) changed)
- **backlog-groom-2-4** — Folded TERMINAL_CREATE_SCRIPT and UNASSIGNED_TERMINAL_SCRIPT out of views.rs into two scoped IIFEs in assets/app.js; Unassigned page's <main> now carries data-unassigned-base so its own poll/reply/keys wiring builds routes from a data attribute instead of a hardcoded string. cargo test --workspace: 1049 passed; cargo clippy --workspace --all-targets: no issues; cargo fmt --check: clean; node --check on app.js: syntax OK. Manual browser check (unautomatable, recorded per home-terminal-header-2 precedent, not run this session): open /_terminal/unassigned with a live pane and confirm it still polls /_terminal/unassigned/<pane>/screen and posts to .../input on Send/Approve/Stage and .../keys on key presses; open a project terminal page and the homepage Terminals tab and confirm New shell/preset buttons still POST to /p/<id>/_terminal/create/pane|agent and reload on success. (2 file(s) changed)
- **backlog-groom-2-5** — Introduced BeeHubCardArgs struct (with lifetime) holding all 15 fields; updated all 22 call sites (2 production, 20 test) to build the struct; HTML byte-identical, cargo test --workspace 1049 passed, fmt+clippy clean (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **backlog-groom-2-1** — `cargo test --workspace green (CI triple). New home_page test: a home render with an empty cross-project board still contains the tab strip and the Terminals tab anchor; a populated board still renders as before. Update any existing test that asserted the tabless early-return to the new contract, not deleted.`
- **backlog-groom-2-2** — `cargo test --workspace green (CI triple). New test: a shell (agentless) pane renders non-empty aria-labels (no "Scroll 's history" / "Reply to " with an empty name); an agent pane's labels are unchanged.`
- **backlog-groom-2-3** — `cargo test --workspace green (CI triple). New view test: a board render from a snapshot with read_errors contains the warning strip with the count; a clean snapshot renders no strip. If the snapshot type needs a read_errors count exposed to the view, thread it minimally.`
- **backlog-groom-2-4** — `cargo test --workspace green (CI triple). All existing terminal/unassigned/create tests stay green (behavior unchanged). Any Rust test asserting the presence of the inline const text is updated to assert the data-attribute/app.js wiring instead. The JS itself has no repo harness: record the manual browser check (Unassigned page still polls its own route; create control still starts a session; homepage Terminals tab unchanged) per home-terminal-header-2 precedent.`
- **backlog-groom-2-5** — `cargo test --workspace green (CI triple) — the existing bee_hub_card / board tests must stay green with identical rendered output, proving the refactor changed nothing observable. fmt + clippy clean (a params struct commonly clears a clippy too_many_arguments lint too).`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work backlog-groom-2` from 5 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/backlog-groom-2.json`. Applied 2026-08-16 from docs/history/backlog-groom-2/promote-proposals.md; the proposal's area bullets and pattern candidates were deliberately not applied — disposition recorded in the decision log (backlog-groom-2/home-board-perf promote disposition).
