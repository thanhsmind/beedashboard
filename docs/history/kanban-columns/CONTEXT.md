# Kanban Columns — Context

**Feature slug:** kanban-columns
**Date:** 2026-08-14
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE | ORGANIZE

## Feature Boundary

Every feature board — the home page's Kanban tab and each project's own
board — shows five columns: Todo, In Progress, Review, Compound, Finished,
where only In Progress renders cards and the other four render dense one-line
rows, and a feature that is stopped on an unapproved gate stays in In Progress
carrying a "Waiting on you" line on its card. It ends at those two boards: the shipped-features
`bee_finished_section` list and the Projects tab are untouched.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | The board renders five columns in this left-to-right order: **Todo, In Progress, Review, Compound, Finished**. The former "Waiting on you" column is removed entirely — no column of that name survives. | The order is the bee lifecycle itself, so reading the board left to right reads the flow of work. |
| D2 | **Todo** holds two kinds, in this order: first, features whose cells are open with none claimed; second, backlog PBIs in state `proposed`. | Planned-but-unclaimed work has had no home on the board at all. |
| D3 | A PBI in Todo renders as a **dense flat row** in the same shape the Finished column already uses (`bee_hub_finished_row`), not as a full card. The row is clickable and links to the bee board of the project that owns the PBI. | A PBI is not yet a feature — it has no description, progress, worktree, or activity to fill a card. |
| D4 | **Review** holds features with an unresolved review candidate. **Compound** holds features whose lane `phase` is `compounding`. **Finished** keeps its existing rule unchanged (`phase == "compounding-complete"` or an archived-cells directory of its own). | — |
| D5 | **In Progress wins every tie.** A feature with live work stays in In Progress even when it has a review candidate waiting; it reaches Review only once it has no live cells left and the candidate is still unresolved. A feature that is the active feature (or carries a live session or granted worktree) is In Progress even when every one of its cells is still open and unclaimed — it never falls to Todo. | Review and Compound are hard-collapsed, so placing running work there would hide it. |
| D6 | ~~Review, Compound, and Finished are hard-collapsed~~ — **superseded by D12.** | Kept for the record; cite D12. |
| D7 | A feature stopped on an unapproved gate stays in **In Progress** and is marked by one small line on its card: the label `Waiting on you` followed by a short reason naming the exact gate — e.g. `Waiting on you — gate shape`. | Waiting for a human is a state of running work, not a separate stage. |
| D8 | The rule that picks the reason text in D7 is the existing Waiting-column rule, carried over unchanged: the current stop gate from `bee_gate_current_stop` (excluding `review`), or a `.bee/HANDOFF.json` that reads as a genuine pause on the active feature. | Reuses proven logic; no new notion of "waiting" is introduced. |
| D9 | All five columns apply to **both** boards that render them: the cross-project board on the home page (`bee_cross_project_features_section`) and each project's own board (`bee_feature_hub_section`). One classification rule serves both — no fold-back layer that keeps the per-project board on three columns. | The two boards already share `bee_classify_features`; splitting the rule would cost a fold-back layer and leave the two boards reading differently. |
| D10 | For placement, **live work narrows**: only `doing` or `stuck` cells count, plus the three existing pulls (the feature is the active one, a live session names its lane, or a granted worktree names it). An `open` cell alone is no longer live work. | Today's rule counts `open` cells as live, which would make D2's Todo branch unreachable — every feature it should catch would be pulled into In Progress first. This is the only reading under which D2 and D5 are both true. |
| D11 | Placement is tested in this order: In Progress, then Finished, then Review, then Compound, then Todo. Intended consequence: a closed feature (`compounding-complete` or archived) stays in Finished even when it still carries an unresolved review candidate. | Preserves every one of today's Finished behaviours; a closed feature reads as closed. |
| D12 | **Supersedes D6.** Four columns — Todo, Review, Compound, Finished — render each item as one dense flat row in the shape Finished uses today: the name (CONTEXT.md title, else the slug), a project label on the cross-project board, and a link. Only **In Progress** renders full cards. All four dense columns page at ten rows exactly as Finished does today — ten rows open, the remainder behind nested `Show N more` disclosures. | A count alone says nothing. A flat row is enough to recognise the work while leaving the horizontal room to In Progress, the one column that earns a card — and it reuses the row and pager that already exist. |

### Agent's Discretion

- The exact CSS class names, chip styling, and grid tracks for a five-column
  board of four dense columns and one card column, so long as D12's row shape
  and D7's label-plus-reason hold.
- Where the "Waiting on you" line sits within the card's existing line order,
  so long as it is one line and reads as part of the card.
- How the two kinds in Todo (D2) are separated visually, if at all.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Dense row | One line per item, in the shape `bee_hub_finished_row` renders today: name, optional project label, a link, and nothing else — no description, progress bar, worktree chip, or activity line. Used by Todo, Review, Compound, and Finished (D12). |
| Live work | Narrowed by D10: `doing` or `stuck` cells, or the globally active feature, or a live session whose lane names the feature, or a granted worktree that names it. `open` cells alone are not live work — that is the change from today's board, which also counted them. |
| Unclaimed open | A feature whose cells exist and are open with no claim on any of them — the Todo condition in D2. |

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance/src/views.rs:2212` — `bee_classify_features`, the single
  function that sorts a feature into a column; today it returns the three-variant
  `BeeHubPlacement` enum at `views.rs:2174`.
- `crates/waggledance/src/views.rs:2650` — `bee_gate_current_stop`, the gate-stop
  helper D8 carries over unchanged.
- `crates/waggledance/src/views.rs:2946` — `bee_hub_finished_row`, the dense flat
  row D3 reuses for PBIs.
- `crates/waggledance/src/views.rs:2679` — `bee_hub_group`, which emits the
  heading, count chip, and body for one column.

### Established Patterns

- Column bodies are plain server-rendered HTML with no JavaScript; the only
  existing collapse anywhere on this board is native `<details>`
  (`bee_hub_finished_more`, `views.rs:3005`). D6 needs neither.
- `BeeFeaturePhase.phase` is a free-form `Option<String>`, not an enum
  (`crates/waggledance-core/src/bee.rs:672`); the classifier compares it against
  string literals, and D4 adds `"compounding"` to that comparison.

### Integration Points

- `crates/waggledance/src/views.rs:2396` — `bee_feature_hub_section`'s HTML
  wrapper, which today interpolates exactly three groups into `.bee-hub__groups`.
- `crates/waggledance/src/views.rs:2799` — `bee_hub_card`, where D7's line slots
  in beside the existing `reason_html` line (`views.rs:2889`).
- The snapshot fields feeding review candidates and PBIs must be confirmed
  present on `BeeSnapshot` — see Deferred To Planning.

## Canonical References

- `docs/history/homepage-tabs/CONTEXT.md` — the two-tab home page this board
  lives inside; the Kanban tab is the only surface this feature changes.

## Outstanding Questions

### Deferred To Planning

- [ ] Does `BeeSnapshot` already carry review candidates and backlog PBIs per
      project, or must the collector be extended? — read
      `crates/waggledance-core/src/bee.rs` for the snapshot's own fields.
- [ ] How is "no claim on any cell" (D2) read from the existing
      `BeeFeatureCellCounts`, which exposes `doing`/`waiting`/`stuck`/`done`/`total`
      but no explicit claim count? — may need a derived rule or a new count.
- [ ] Whether five columns still fit the `minmax(260px, 1fr)` auto-fit grid at
      `views.rs:1688`, given three of them are now heading-only.

## Deferred Ideas

- Remembering a viewer's expand/collapse choice across page loads — the four
  dense columns use the same stateless native disclosures Finished uses today
  (D12), and nothing is remembered between loads.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
