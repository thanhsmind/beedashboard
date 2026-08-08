promote proposal for work item "project-suggestions" (docs/history/project-suggestions/CONTEXT.md + docs/history/project-suggestions/plan.md) — 5 capped cell(s): project-suggestions-1, project-suggestions-2, project-suggestions-3, ps-1, ps-2
anchor: history — docs/history/project-suggestions/CONTEXT.md, docs/history/project-suggestions/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/project-suggestions/delivery.md

---
type: bee.delivery
title: project-suggestions — delivery
description: "Delivery record proposed by bee knowledge promote for work item project-suggestions: 5 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-08
bee:
  id: project-suggestions-delivery
  lifecycle: active
  areas: [web-interface]
  required_context: [docs/history/project-suggestions/CONTEXT.md, docs/history/project-suggestions/plan.md]
  sources: [docs/history/project-suggestions/CONTEXT.md, docs/history/project-suggestions/plan.md, .bee/cells/archive/project-suggestions/project-suggestions-1.json, .bee/cells/archive/project-suggestions/project-suggestions-2.json, .bee/cells/archive/project-suggestions/project-suggestions-3.json, .bee/cells/archive/project-suggestions/ps-1.json, .bee/cells/archive/project-suggestions/ps-2.json]
---

# project-suggestions — Delivery

## What shipped

- **project-suggestions-1** — Added suggested_projects computing unregistered folders with running herdr sessions, wired into index_page behind terminal_family_enabled, with a rendered suggestion block reusing the register route and route tests covering every must-have truth (3 file(s) changed)
- **project-suggestions-2** — Closed the raw-path-containment disclosure gap: suggested_projects now drops any candidate contained under a registered project's root by component-wise raw containment (new paths_boundary::is_contained_in_root), catching deleted directories, dot-dot cwds, and missing project roots that project_panes alone misses; 719 workspace tests pass. (2 file(s) changed)
- **project-suggestions-3** — Drop traversal cwds outright in suggested_projects, closing the sibling-dot-dot disclosure and dead-button leak; repoint the empty-cwd test at rendered row count; add sibling-prefix and symlink-onto-deny-list route tests (1 file(s) changed)
- **ps-1** — Reworked project-suggestions ps-1 to D6 agent-only complement via unassigned_panes reuse, D2 path+count row type with byte-identical escaping, D1 trailing-slash dedup, bytewise sort, corrected stale unassigned_visible comment, and the leak-test's three-phase re-expression (2 file(s) changed)
- **ps-2** — Register-from-suggestion flow covered: happy path now asserts the suggestion row is gone, plus new duplicate-banner and ON-state no-path pin tests; views.rs form was already correct from ps-1 (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **project-suggestions-1** — `cargo test --workspace`
- **project-suggestions-2** — `cargo test --workspace`
- **project-suggestions-3** — `cargo test --workspace`
- **ps-1** — `cargo test --workspace`
- **ps-2** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work project-suggestions` from 5 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/project-suggestions/CONTEXT.md`, `docs/history/project-suggestions/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "project-suggestions" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-08T10:10:09.244Z), the work item declares no bee.areas.

area web-interface:
  - [project-suggestions-1] Added suggested_projects computing unregistered folders with running herdr sessions, wired into index_page behind terminal_family_enabled, with a rendered suggestion block reusing the register route and route tests covering every must-have truth — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/archive/project-suggestions/project-suggestions-1.json)
  - [project-suggestions-2] Closed the raw-path-containment disclosure gap: suggested_projects now drops any candidate contained under a registered project's root by component-wise raw containment (new paths_boundary::is_contained_in_root), catching deleted directories, dot-dot cwds, and missing project roots that project_panes alone misses; 719 workspace tests pass. — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/archive/project-suggestions/project-suggestions-2.json)
  - [project-suggestions-3] Drop traversal cwds outright in suggested_projects, closing the sibling-dot-dot disclosure and dead-button leak; repoint the empty-cwd test at rendered row count; add sibling-prefix and symlink-onto-deny-list route tests — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/archive/project-suggestions/project-suggestions-3.json)
  - [ps-1] Reworked project-suggestions ps-1 to D6 agent-only complement via unassigned_panes reuse, D2 path+count row type with byte-identical escaping, D1 trailing-slash dedup, bytewise sort, corrected stale unassigned_visible comment, and the leak-test's three-phase re-expression — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/archive/project-suggestions/ps-1.json)
  - [ps-2] Register-from-suggestion flow covered: happy path now asserts the suggestion row is gone, plus new duplicate-banner and ON-state no-path pin tests; views.rs form was already correct from ps-1 — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/archive/project-suggestions/ps-2.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 5 capped cell(s) mined, 1 delivery draft, 5 area bullet(s), 0 pattern candidate(s), 0 file(s) written.