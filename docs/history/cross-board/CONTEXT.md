# Cross-Project Board — Context

**Feature slug:** cross-board
**Date:** 2026-08-12
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE

## Feature Boundary

The home page at `/` becomes a roll-up of bee state across every registered
project that has a `.bee/` directory: a cross-project Live strip, a
cross-project Features board with the same three columns the per-project board
uses, and the existing project list moved below them. It ends at the home page —
the per-project board at `/p/:id/_bee` is not changed, and no new route is added.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | The cross-project board lives on the home page `/`. Order down the page: Live (cross-project), Features (cross-project), then the existing project list. | The user chose one page over a second destination, so "what is waiting on me" is answered without navigating. |
| D2 | No new route is created and `/p/:id/_bee` is left exactly as it is. | The per-project board stays the place to go deep on one project. |
| D3 | The Features section keeps the same three columns, in the same order and with the same names: Waiting on you, In Progress, Finished. | The user asked for the per-project presentation, applied across projects. |
| D4 | Features are listed flat inside each column — never grouped into per-project blocks and never one row per project. | A flat list answers "what is waiting on me" first; the project is a detail on the item, not the organising axis. |
| D5 | Every feature card and every Finished row carries a label naming the project it belongs to. | Without it a flat cross-project list is unreadable. |
| D6 | ~~The Finished column is one shared ship timeline across all projects — most recently finished first.~~ **Superseded by D10.** | Locked on the premise that a reliable ship timestamp exists; it does not. |
| D10 | The Finished column orders in two blocks. First, every feature that has a `cycle_time.ended_at`, most recently finished first, each row showing that time. Behind them, every feature without one, ordered alphabetically by feature name across all projects. The 10-row cap and "Show 10 more" of D7 apply to the combined sequence. | `cycle_time.ended_at` is `Option` and only exists when every live cell of the feature carries both `claimed_at` and `capped_at` (`bee.rs:342-344`; `compute_velocity` at `bee.rs:2346` already filters the missing case as routine). A pure timeline would bury most features in an unexplained tail; the per-project board's own order is alphabetical (`views.rs:1901`). |
| D7 | The Finished column shows 10 rows, with the rest behind a "Show 10 more · N left" control that matches the per-project board's existing behaviour. | One project alone already has 105 finished features; the page must stay light, and the user already knows this control. |
| D8 | A project appears in the cross-project sections only when it is registered AND its root has a `.bee/` directory. Registered projects without `.bee/` still appear in the project list below, unchanged. | This is the same qualification rule the per-project bee surface already states in `docs/specs/bee-cockpit.md`. |
| D9 | When no registered project qualifies, the Live and Features sections are absent from the page entirely and `/` reads exactly as it does today. | A person with no bee projects must not be shown two empty shells. |

### Agent's Discretion

- The exact wording and visual form of the project label on a card or row (D5),
  within the existing appearance system — no new colour or component vocabulary.
- Whether the counts beside each column heading are per-column totals across all
  projects, provided the number shown always matches the items the column holds.
- How the roll-up is read and cached, provided the page's correctness rules
  above hold and reading N projects does not make `/` visibly slower than the
  per-project board is today.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Cross-project board | The Live + Features roll-up now hosted on `/`. |
| Per-project board | The existing page at `/p/:id/_bee`, unchanged by this feature. |
| Qualifying project | A registered project whose root contains `.bee/` (D8). |
| Project label | The project name shown on a cross-project card or row (D5). |

## Specific Ideas And References

- The user supplied a screenshot of the current per-project board — Live strip
  with one worktree row, then Features with the three columns, the Finished
  column dense-listed and capped by "Show 10 more · 95 left". That screenshot is
  the target presentation; this feature reproduces it over many projects.

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/mdview-core/src/bee.rs:932` — `read_snapshot(root)` reads one project's
  `.bee/` and returns `BeeSnapshot`; returns `BeeSnapshot::absent()` when the
  directory is missing. The roll-up is N calls to this, one per qualifying project.
- `crates/mdview/src/views.rs:1872` — `bee_feature_hub_section` builds the three
  column groups and the classification rules that decide which column a feature
  lands in (doc comment at `views.rs:1765-1871`).
- `crates/mdview/src/views.rs:2121` / `views.rs:2198` — `bee_hub_card` and
  `bee_hub_finished_row`, the two item renderers that must gain the project label.
- `crates/mdview/src/views.rs:2217` / `views.rs:2233` — `bee_hub_finished_rows`
  and `bee_hub_finished_more` already implement D7's paging with nested
  `<details>` and no JavaScript.
- `crates/mdview/src/views.rs:1751` — `bee_live_strip_section`, built from
  `snapshot.sessions` and `snapshot.worktrees`.

### Established Patterns

- Views are hand-built `format!` strings in `crates/mdview/src/views.rs`; there
  is no template engine in the view path.
- Bee page composition: `bee_board_page` (`views.rs:1360`) chains top bar, Live
  strip, feature hub, finished section, panels.

### Integration Points

- `crates/mdview/src/server.rs:584` — `index_page`, the `/` handler, which today
  builds a project list and never touches bee state.
- `crates/mdview/src/views.rs:104` — `project_list_page`, the view `/` renders;
  this is what moves below the new sections.
- `crates/mdview/src/server.rs:1257` — `is_bee_project`, the existing
  `.bee/`-presence check that implements D8's second condition.
- `crates/mdview-core/src/engine.rs:202` — `list_projects`, the registry read
  that supplies the set to roll up.

## Canonical References

- `docs/specs/bee-cockpit.md` — the read-only bee surface: which projects
  qualify, what each column means, how features are classified.
- `docs/specs/web-interface.md` — the nav chrome `/` sits inside.
- `docs/specs/reading-map.md` — where each area's code lives.

## Outstanding Questions

### Deferred To Planning

Both questions this section opened are now answered; the answers are recorded
here so planning does not re-measure them.

- **Registry size.** 10 registered projects, 8 of them qualifying under D8 (the
  other two are stale worktree registrations with no `.bee/`). Their `.bee/`
  stores hold 199 cells and ~308 `docs/history/<feature>/` directories between
  them, the largest single project carrying 206 feature directories.
- **Cost of the roll-up.** `read_snapshot` (`bee.rs:932`) is synchronous
  filesystem work — roughly ten fixed reads plus one read per cell plus one
  directory read per feature — and today both of its callers
  (`server.rs:1268`, `server.rs:3081`) invoke it directly on the async task with
  no cache. Eight of those on `/` is the shape planning must design against.
- **Ship timestamps.** Answered and locked as D10.

## Deferred Ideas

- Filtering or searching the cross-project board (by project, by phase) — the
  first version presents everything; no filter was asked for.
- A cross-project Backlog and review panel to match the per-project
  `bee_panels_section` — the user named Live and Features only.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
