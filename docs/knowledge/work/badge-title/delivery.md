---
type: bee.delivery
title: badge-title — delivery
description: "Delivery record for work item badge-title: terminal badges name what a pane is working on, not just what it is running."
timestamp: 2026-08-18
bee:
  id: badge-title-delivery
  lifecycle: active
  areas: [bee-cockpit, agent-terminal]
  required_context: [docs/specs/bee-cockpit.md, docs/specs/agent-terminal.md]
  sources: [.bee/lanes/badge-title.json, .bee/cells/archive/badge-title/badge-title-1.json]
---

# badge-title — Delivery

## What shipped

A row of terminal badges told you a pane's state and which program it ran —
and stopped there. With several panes running the same program, the badges
were indistinguishable, and the one thing that separated them, the pane's own
title, was visible only after opening the agents drawer.

Each badge now carries a third piece: the pane's terminal title, printed
after the program name. The order is state, program, title.

The title is dropped when it would say nothing new — empty, or identical to
the program name. A pane running a bare shell under the title "shell" reads
exactly as it did before; a pane whose title names the task it is on now says
so on the badge itself.

It is the same title the agents drawer already shows, so the two surfaces
cannot disagree, and it renders on both places the badge row appears: the
project list and the feature hub's cards.

## Verify

`cargo fmt --all --check && cargo clippy -p waggledance --all-targets -- -D
warnings && cargo test -p waggledance` green. New cases cover the title
rendering after the program span when it says something new, and being
skipped when empty or redundant with the program; the existing case asserting
that the hub card's badges match the project list's markup shape keeps both
surfaces in step.

## Deviations

None recorded.

## Pointers

- `crates/waggledance/src/views.rs:546` — `terminal_badges_nav`, the one
  renderer; the badge order is at `views.rs:572-578` and the non-redundancy
  guard `!p.title.is_empty() && p.title != p.kind` at `views.rs:564`.
- `TerminalPaneView.title`, populated in `project_panes` and
  `unassigned_panes`; styled as `proj-row__badge-title` in `app.css`.
- Callers: `project_badges` (`views.rs:523`, from `project_list_main`) and
  `bee_hub_card` (`views.rs:3817`).
- Tests: `views.rs:10626`, `views.rs:10662`, `views.rs:10492`.

## Provenance

Written from the capped trace of `badge-title-1`, verified against the
shipped source. The badge's contents are the decision logged 2026-08-18
(6b39db89).
