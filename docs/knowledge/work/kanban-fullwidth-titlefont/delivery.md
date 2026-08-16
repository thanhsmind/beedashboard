---
type: bee.delivery
title: kanban-fullwidth-titlefont — delivery
description: "Delivery record for work item kanban-fullwidth-titlefont: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: kanban-fullwidth-titlefont-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [docs/specs/bee-cockpit.md]
  sources: [.bee/lanes/kanban-fullwidth-titlefont.json, .bee/cells/kanban-fullwidth-titlefont-1.json]
---

# kanban-fullwidth-titlefont — Delivery

## What shipped

- **kanban-fullwidth-titlefont-1** — Board pages (home Kanban tab, per-project bee board) drop the 1200px page clamp on desktop ≥1240px and span the full viewport; kanban card titles switch to the system sans face. CSS-only change; detail pages keep the reading column.

## Verify

- **kanban-fullwidth-titlefont-1** — `cargo test --workspace` green; app.css carries both new rules; views.rs and server.rs untouched.

## Deviations

None recorded in the capped cell traces.

## Provenance

Mined by `bee knowledge promote --work kanban-fullwidth-titlefont` from the capped cell trace; reviewed and applied at the 2026-08-16 compounding pass. Area sync landed in docs/specs/bee-cockpit.md ("Where it appears").
