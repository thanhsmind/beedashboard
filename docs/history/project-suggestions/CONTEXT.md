# Project Suggestions — Context

**Feature slug:** project-suggestions
**Date:** 2026-08-08
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE

## Feature Boundary

The Projects page gains a suggestion block: every folder that an agent-backed
herdr pane is working in, but that no registered project covers, appears as a
suggestion with a one-click Register button. The feature ends at the block and
its button — registration itself, validation, and project rows are the existing
machinery, unchanged.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | Suggestions come from herdr: every session whose folder sits under no registered project becomes a suggestion pointing at that pane's own working directory, exactly as reported, with no walk up to a repository root. | The pane's cwd is the one fact herdr actually reports; guessing a repo root would invent structure. |
| D2 | The suggestion block shows full filesystem paths; rows for projects that are already registered still never do. (Supersedes web-interface R5 in part.) | The path is the suggestion — the user must see exactly what would be registered. |
| D3 | The block is gated on `terminal.enabled` alone, not additionally on `unassigned_enabled`, even though it reads the same set of panes that group does. (Narrows toa-4/D9 for one surface.) | — |
| D4 | Suggestions group by folder: one suggestion row per distinct unregistered cwd, showing how many agent panes run inside it — never one row per pane. | Avoids duplicate rows when several panes share a working directory. |
| D5 | Each suggestion carries a Register button that posts the suggested path straight to the existing `/api/projects/register` route; failures surface through the same `register_error` banner codes as the manual form. | Reuses the validated route with its deny-list and bounded-scan checks unchanged; no new endpoint, no client JS. |
| D6 | Only agent-backed panes produce suggestions, matching how the Unassigned group is computed; shell-only panes never do. | Keeps the block consistent with `unassigned_panes` and low-noise. |
| D7 | Suggestions are stateless and cannot be dismissed: the block is recomputed on every page render and a suggestion disappears only when its pane closes or its folder becomes registered. | No new persistence; matches the page's synchronous render-per-request model. |

### Agent's Discretion

Visual layout of the block (card vs. list styling, ordering of rows, wording of
the count label) — constrained to the page's existing card/form patterns and to
D2's full-path display.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| suggestion | One distinct unregistered working directory, aggregated over the agent-backed panes reporting it — not a pane and not a repository root. |

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/mdview/src/server.rs:1861` — `unassigned_panes(snapshot, projects)` computes exactly the complement set (agent panes under no registered project) this feature aggregates; note its fail-closed rule: one unconstructable project boundary empties the whole group.
- `crates/mdview/src/server.rs:716` — `register_project` handler with `validate_register_path` (`server.rs:763`); D5 posts here unchanged.
- `crates/mdview/src/views.rs:229` — `register_error_message` banner codes; D5 failures reuse them.

### Established Patterns

- Presence-only card block — `views.rs:169-179` (Unassigned card): the visual template for the suggestion block.
- Plain HTML form POST with redirect, no client JS — `views.rs:211-219` (add-project form): the Register button is this form with the path prefilled as a hidden value.

### Integration Points

- `crates/mdview/src/server.rs:343-401` — `index_page` already takes a herdr snapshot when the terminal family is on; the suggestion computation hangs off that same snapshot.
- `crates/mdview/src/views.rs:91` — `project_list_page(...)` grows the suggestion block parameter.
- `crates/mdview/src/server.rs:1114` — `terminal_family_enabled` is the D3 gate.

## Canonical References

- `docs/history/projects-home/` — the register route, deny-list containment (D9b), and Projects-page rows this feature builds on.

## Outstanding Questions

### Resolve Before Planning

None.

### Deferred To Planning

- [ ] Whether the pane's `cwd` or `foreground_cwd` (unix live dir) feeds the suggestion — read how `project_panes`/`unassigned_panes` already resolve the fallback and mirror it. (Answered in plan.md: display reads `Pane.cwd` only.)
- [ ] Ordering of suggestion rows (path-sorted vs. pane-count) — agent's discretion within the SEE block; pick during planning. (Answered in plan.md: sorted by path, bytewise.)

## Deferred Ideas

None came up.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.

Note for workers in this worktree: this worktree carries earlier commits
(16303c5..accec89) implementing a superseded version of this feature (S1–S3,
cells project-suggestions-1/2/3), including shell-pane suggestions that D6 now
forbids. The current plan.md is the authority; rework the existing code to it.
The frozen approved plan lives in the main checkout at
/home/thanhsmind/projects/goglbe/beedashboard/docs/history/project-suggestions/plan.md
(read-only from here); the worktree's own committed plan.md is the superseded one.
