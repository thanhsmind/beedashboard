# Collapsible In Progress cards — Context

**Feature slug:** card-collapse-inprogress
**Date:** 2026-08-15
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE

## Feature Boundary

Every In Progress card on the feature board renders collapsed by default —
its feature name and a disclosure chevron, with its terminal badges still
visible at the card's foot — and expands its existing detail body when the
user clicks the card's header. It ends at the In Progress card: no other
column, no other card renderer, and no change to what the expanded body
says.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | An In Progress card is collapsed by default. Collapsed, it shows exactly two things: a header row carrying the card's name (its CONTEXT title when it has one, else its slug) plus a disclosure chevron, and — when the feature's checkout has terminal panes — the existing terminal-badge `<nav>` at the card's foot. | The In Progress column grows long when several features run at once; the user scans by name. The running terminals are the one thing worth seeing without opening a card. |
| D2 | Everything else the card renders today moves into the expandable body: the project/worktree subtitle, the description, the progress bar and its `N/M cells done` label, the reason line, and the last-activity line. | — |
| D3 | Clicking anywhere on the header row toggles the card open or closed. The card is no longer an `<a>` wrapping its whole content. | A single element cannot be both a whole-card link and a toggle. A large hit area is what makes this usable on a phone. |
| D4 | The link to the feature's own detail page becomes its own row at the top of the expanded body, reading `Feature detail` with a trailing arrow — the shape the user's reference mock uses for `Stream settings →`. | The detail page must stay one click away once the card is open; it just no longer owns the whole card. |
| D5 | Both boards get this card: the homepage cross-project Kanban tab and each project's own bee board. Both `bee_hub_card` call sites render the same collapse markup. | One renderer, one card. A second card shape would drift. |
| D6 | Open/closed state is never remembered. Every page load renders every In Progress card collapsed. No `localStorage`, no `sessionStorage`. | The board does not poll-refresh itself, so nothing snaps a card shut mid-read; dropping the state keeps this a pure server render with no new client state to migrate. |
| D7 | The expanded body keeps today's presentation exactly — the same subtitle, the clamped two-line description, the green progress bar with its label, the activity and reason lines. It is NOT rewritten into the reference mock's `label ······ value` leader rows. | The mock is a reference for the collapse behaviour, not for how this card presents its data. A progress bar reads faster than a sentence. |

### Agent's Discretion

The disclosure mechanism itself (native `<details>`/`<summary>` versus a
button plus a class toggle), the chevron's own markup and its rotation, and
the exact CSS that hides the collapsed body are the agent's to choose —
subject to D6 (no persisted state) and to the header row staying keyboard
reachable and carrying an accurate expanded/collapsed state for screen
readers.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Header row | The always-visible top line of an In Progress card: its name plus the chevron. The click target for the toggle (D3). |
| Body | Everything the card reveals when expanded (D2), plus the `Feature detail` link row (D4). |
| Badges | The existing terminal-pane `<nav>` at the card's foot. Outside the body — visible collapsed and expanded alike (D1). |

## Specific Ideas And References

- The user's reference screenshot (a card titled `platform_production_event`
  with a chevron in its header, a `Stream settings →` row, and
  `label ······ value` rows beneath) supplies the **collapse behaviour and
  the header/link-row shape only**. Its leader-row data presentation is
  explicitly out (D7).

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance/src/views.rs:3234` — `bee_hub_card`, the single
  renderer for every In Progress card. Today it emits
  `<div class="fg-card bee-hub__shell">` wrapping one `<a class="bee-hub__card">`
  plus a sibling badge `<nav>`.
- `crates/waggledance/src/views.rs:1834` — `bee_hub_style()`, the inline
  `<style class="bee-hub-theme">` block that owns every `bee-hub__*` rule.
  These rules are NOT in `assets/app.css`, which has zero `bee-hub` matches.
- `crates/waggledance/assets/atelier/components.css:51` — `.fg-card`, the
  box paint the shell carries.

### Established Patterns

- Native `<details>`/`<summary>`, already used for collapsed content on this
  very board: the Finished section at `views.rs:3655`
  (`bee-done-details` / `bee-done-summary`, collapsed by default, no `open`
  attribute) and the row-overflow pager at `views.rs:3485`
  (`bee-hub__more`). Neither is driven by any JavaScript.
- The class-toggle alternative, if a `<details>` proves wrong: `.chap-folders`
  (`app.js:114-144` for the client-built copy, `app.js:171-193` for the
  server-rendered one) — a `<button class="chap-folders__bar" aria-expanded>`
  plus an `.is-open` class and a `0fr→1fr` grid height reveal
  (`app.css:666-729`). Note that pattern persists through `sessionStorage`,
  which D6 forbids here.
- `app.js` has **no** delegated `data-*` click dispatcher to plug a new
  toggle into; the only document-level registry is the `.js-menu`
  outside-click closer at `app.js:1722-1745`, which is keyed on a hidden
  checkbox and DOM containment.

### Integration Points

- `crates/waggledance/src/views.rs:2680` — the per-project bee board's call
  (`bee_render_hub_section`), passing `project_label: None`,
  `project_color: None`, and an empty `panes` slice.
- `crates/waggledance/src/views.rs:2907` — the homepage cross-project Kanban
  tab's call, passing a real project label, colour slot, and panes.
- The seven `bee_hub_card` markup tests at `views.rs:7916, 7992, 8064, 8098,
  8128, 8165, 8189`, plus the style test at `views.rs:8242` and the
  page-body assertions at `server.rs:4682, 7774`, all read today's markup
  and will need updating with it.

## Canonical References

- `docs/knowledge/work/kanban-columns/delivery.md` — why In Progress is the
  one column that still renders cards at all.
- `docs/knowledge/work/project-color-identity/` — the accent border and the
  project/worktree subtitle this card carries.

## Outstanding Questions

### Deferred To Planning

- [ ] Whether `<details>`/`<summary>` can carry the chevron and the accent
      border without fighting `.fg-card`'s own box paint, or whether the
      button-plus-class pattern is needed — answered by building the
      `<details>` version first and looking at it.

## Deferred Ideas

- Collapsing the card on the feature-detail page and the other four dense
  columns — those columns render one-line rows already, so there is nothing
  to collapse.
- Remembering which cards the user left open — explicitly ruled out for now
  by D6; revisit only if the board gains a poll refresh.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
