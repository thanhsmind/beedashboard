---
type: bee.delivery
title: terminals-tab-project-scope — delivery
description: "Delivery record for work item terminals-tab-project-scope: the Terminals tab's pane switcher lists only panes from the selected pane's own project."
timestamp: 2026-08-16
bee:
  id: terminals-tab-project-scope-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [docs/specs/agent-terminal.md]
  sources: [.bee/lanes/terminals-tab-project-scope.json, .bee/cells/terminals-tab-project-scope-1.json]
---

# terminals-tab-project-scope — Delivery

## What shipped

- **terminals-tab-project-scope-1** — Scope the Terminals tab switcher to the active pane's project: the pane_bar switcher (wide strip and the narrow-screen collapsed menu alike) filters the full pane inventory down to panes sharing the effective pane's own project, matched on project identity rather than the display-only label; a pane with no project lists only the other project-less panes. Selection resolution (first-pane fallback, vanished-pane check) unchanged. (1 file(s) changed)

## Verify

Capped against the CI triple: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.

## Deviations

None recorded in the capped cell trace.

## Provenance

Proposed by `bee knowledge promote --work terminals-tab-project-scope` from 1 capped cell trace. Accepted at the compounding pass on 2026-08-16. The work item declared no areas, but the behavior is user-observable and was NOT yet stated in the living spec — the switcher-scoping rule was merged into `docs/specs/agent-terminal.md` at this pass (the Agents drawer remains the cross-project view). No pattern candidates were proposed.
