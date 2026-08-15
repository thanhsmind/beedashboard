# In Progress first, and busiest first — Context

**Feature slug:** inprogress-priority-order
**Date:** 2026-08-15
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE

## Feature Boundary

On a narrow screen the In Progress column moves to the top of the stacked
board, and inside that column the cards order themselves by what is
actually happening: a feature with a working terminal outranks one without,
and newer activity outranks older. It ends at the In Progress column — the
other four columns keep both their position and their order.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | On a narrow screen the In Progress column renders first, above Todo; the other four keep their relative order (In Progress, Todo, Review, Compound, Finished). Reuse the board's existing `max-width: 700px` breakpoint — no new breakpoint. Wide screens are unchanged. | Stacked, the one column that still carries cards was sitting below Todo. |
| ~~D2~~ | ~~Sort by working terminal, then activity.~~ **Superseded by D7.** | — |
| ~~D3~~ | ~~"A working terminal" means `status == "working"` only.~~ **Superseded by D7.** | — |
| D4 | The homepage Kanban tab's In Progress column drops its per-project grouping: one flat list sorted by D2 across every project, never project-by-project. The four dense-row columns keep the grouping they have today. | A global activity sort and a per-project grouping cannot both hold. A card still names its project through its left accent border and its subtitle. |
| D5 | The per-project bee board's In Progress cards receive terminal pane data too — so they render terminal badges and obey D2's terminal priority exactly as the homepage cards do. | Without pane data the terminal half of D2 would silently do nothing on that board, leaving two boards with two different orders. |
| D6 | The sort applies to the In Progress column only. Todo, Review, Compound and Finished keep today's order, including Finished's own newest-shipped-first sort. | — |
| D7 | In Progress cards sort by three priority tiers: (1) carrying at least one pane whose `status` is `"blocked"`, (2) carrying at least one pane whose `status` is `"working"`, (3) everything else. Within a tier: newest `last_activity` first, a card with no recorded activity last, then feature name A→Z. `idle`, `done`, `unknown` and `shell` never earn a tier. Supersedes D2 and D3. | A blocked agent is standing still waiting for the user — more urgent than one running on its own. Same `blocked > working > rest` ranking the Agents drawer already uses (`terminals_status_rank`, `server.rs:3014`). |
| D8 | A card carrying at least one `"blocked"` pane renders an extra reason line reading exactly `Waiting on you — a terminal is blocked`, in the card's existing italic `bee-hub__reason` style. A card that already carries a gate/handoff reason line shows BOTH, with the gate line first. | Two different things can be waiting on the user at once; neither should swallow the other. |

### Agent's Discretion

How D1 is achieved (a CSS `order` on the In Progress group inside the
existing media query versus reordering the markup and re-ordering it back
on wide screens) is the agent's call, provided wide-screen rendering is
unchanged and no new breakpoint appears. Likewise how pane data reaches the
per-project board for D5, provided no new HTTP round trip is added to draw
the board.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Blocked terminal | A `TerminalPaneView` whose `status` field is the exact string `"blocked"` (D7, D8). |
| Working terminal | A `TerminalPaneView` whose `status` field is the exact string `"working"` (D7). |
| Activity | The card's existing `last_activity` — the newest `claimed_at`/`capped_at` across the feature's cells, an RFC 3339 string or `None`. |
| Narrow screen | The board's existing `max-width: 700px` media query, the same one that already collapses the five tracks into one column. |

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance/src/views.rs:1947` — `.bee-hub__groups`, the five-track
  grid, and `views.rs:2170-2176`, the `max-width: 700px` query that already
  collapses it to `1fr`. No `order` property exists anywhere in `views.rs`
  or `assets/app.css` today.
- `crates/waggledance/src/views.rs:3606` — `bee_hub_latest_activity`, which
  already produces the `Option<String>` D2 sorts on, and
  `views.rs:3084` — `bee_hub_feature_cells`, its input.
- `crates/waggledance/src/server.rs:2823` — `project_feature_panes`, the
  existing per-project feature→panes join D5 needs.
- `crates/waggledance/src/server.rs:3014` — `terminals_status_rank`, the
  precedent for treating `"working"` as its own checked status.

### Established Patterns

- The one existing card-ordering sort to imitate:
  `cross_project_finished_orders_timed_newest_first_then_untimed_alphabetically`
  (`views.rs:7620`) proves the newest-first-then-untimed-alphabetically
  shape D2 asks for; the Finished re-sort it covers lives at
  `views.rs:3034-3039`.
- Each group `<div>` already carries `data-hub-group="{key}"`
  (`views.rs:3119`), so the In Progress group is selectable from CSS
  without any markup change.

### Integration Points

- `crates/waggledance/src/views.rs:2541` — `bee_classify_features`, which
  sorts features by name (`views.rs:2555-2556`) and pushes
  `BeeHubPlacement::InProgress` in that order; today's alphabetical card
  order is a side effect of it, and no In Progress sort exists.
- `crates/waggledance/src/views.rs:2801-2830` — the per-project board's
  five-group template, and `views.rs:3047-3076` — the homepage Kanban's
  identical one.
- `crates/waggledance/src/views.rs:2899-3005` — the cross-project builder
  that appends each project's In Progress cards project-by-project; D4
  replaces that with one sorted list.

## Canonical References

- `docs/history/card-collapse-inprogress/CONTEXT.md` — the collapsed card
  this work reorders. That feature is green in its worktree but not yet
  merged to main; this work builds directly on it.
- `docs/knowledge/work/kanban-columns/delivery.md` — why In Progress is the
  one column that still renders cards.

## Outstanding Questions

### Deferred To Planning

- [ ] Whether the per-project board's caller (`bee_render_hub_section` and
      whatever builds its arguments in `server.rs`) can reach
      `project_feature_panes` without a new query — answered by reading that
      call path.

## Deferred Ideas

- Sorting the four dense-row columns by activity — out by D6.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
