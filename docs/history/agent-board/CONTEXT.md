# Agent Board (Kanban) — Context

**Feature slug:** agent-board
**Date:** 2026-08-11
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE

## Feature Boundary

Replace the by-phase board section on `/p/:id/_bee` with a Kanban-style agent
board (Backlog → Todo → In Progress → Review → Done) whose cards are cells with
agent badges, answering "which agent is doing what, what's done, what's stuck"
at a glance — display-only, no write path to `.bee/`.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | V1 is display-only: the board surfaces waiting-on-user items and agent activity but never writes to `.bee/`. The bee-board-pm D4 read-only guarantee (byte-identical `.bee/` tree tests) stays intact. On-board Approve/Reject buttons are deferred, not designed in. | A write path would race live agents; separate architectural decision. |
| D2 | Card unit is the cell (small task), not the feature. Feature name is card metadata. | User wants per-task visibility; board density accepted. |
| D3 | Agent axis = agent badge on each card; columns stay status-based. No per-agent swimlanes in V1. | Glance columns for progress, badge for who. |
| D4 | The "Review" column means **waiting on YOU**: pending gates, open questions, paused handoffs. It is never an automatic independent-review queue — bee-board-pm D7 (review is user-invoked) is preserved by this framing. | Answers the "which task is stuck on me" pain directly. |
| D5 | The Kanban board **replaces** the by-phase board section as the main board on `/p/:id/_bee`. The feature-phase view retires (supersedes bee-board-pm bbp-11's board layout, not its data readers). Per-feature detail stays on feature pages. | One board, no duplication. |

### Agent's Discretion

Exact column-to-state mapping details (which `.bee` states feed Backlog/Todo/
In Progress/Done), card layout, and empty-state wording — within the constraints
of D1–D5 and the existing bee-board-pm decisions (D1 English labels, D5 section
order, D6 attention list, D9 no absolute paths).

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Review column | Everything waiting on the user's decision (gates, questions, paused handoffs) — NOT independent-review candidates. |
| Agent badge | The worker/session identity currently holding a cell, rendered on its card. |

## Specific Ideas And References

- User's mental model: managing parallel AI agents like a dev team on an
  Agile/Kanban board — agents self-claim tasks, update progress, important
  actions pause for Approve/Reject. Self-claim and approval already happen in
  bee via CLI; V1's job is to *show* that truthfully, not to operate it.

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/mdview/src/views.rs:2101-2258` — `bee_phase_board_section()` + `bee_phase_card()`: the section being replaced; card/column CSS patterns to reuse.
- `crates/mdview-core/src/bee.rs:1129` — `compute_phase_board()`: lanes ∪ active-feature reader; pattern for a new cell-centric reader.
- `crates/mdview/src/views.rs:1542-1558` — `bee_headline_kpis()`: Doing/Waiting/Stuck/Done buckets over `.bee/cells/*.json` (D7 cell-status buckets) — closest existing state mapping to Kanban columns.
- `crates/mdview/src/views.rs:1585-1661` — `bee_working_now_card()` / `running_workers`: live worker↔cell join, source for agent badges.
- `.bee/sessions/*.json`, `.bee/backlog.jsonl`, `.bee/HANDOFF.json` — existing readers for sessions, backlog (Backlog column), pauses (Review column).

### Established Patterns

- Read-only snapshot: every number from reading `.bee/**`, never executing `bee` (bee-board-pm D4; enforced by `*_read_never_writes_the_fixtures_bee_tree` tests).
- English labels only (bee-board-pm D1); cards link to existing cell/feature detail pages, no drawers (bee-board-pm D3); honest empty states, sections never disappear (bee-board-pm D5); no absolute paths rendered (bee-board-pm D9).
- Targeted WS reload: file pages reload on own-path match, project-scoped pages reload on any project change (targeted-reload decision) — the board page is project-scoped.

### Integration Points

- `crates/mdview/src/server.rs:981` — `bee_board()` handler for `GET /p/:id/_bee`.
- `crates/mdview/src/views.rs:1289` — `bee_board_page()` section order (bee-board-pm D5 fixed order: stepper → KPIs → working-now/attention → board → panels).

## Canonical References

- `docs/history/bee-board-pm/CONTEXT.md` — prior board decisions (D1, D3, D4, D5, D6, D7, D9) that stay binding except where D5 here supersedes the layout.
- `docs/specs/bee-cockpit.md` — read-only guarantees ("It never writes to a project's store").

## Outstanding Questions

### Deferred To Planning

- [ ] Exact column mapping — proposal to validate against real `.bee` data:
  Backlog = backlog PBIs (proposed/open), Todo = open unclaimed cells,
  In Progress = claimed cells + live worker, Review = waiting-on-user set (D4),
  Done = capped cells (recent window) — investigation: check state fields
  actually available in `mdview-core::bee` readers, incl. whether a claimed
  cell's holder (agent identity) is exposed to views.
- [ ] Where do blocked/stuck cells sit — In Progress with a stuck marker, or
  surfaced only via the attention list? (bee-board-pm D6 interaction.)

## Deferred Ideas

Out-of-scope ideas captured during shaping. Not lost, not planned.

- On-board Approve/Reject buttons (a real write path to bee, with a
  concurrency-safety story) — V2 candidate, needs its own shaping (per D1).
- Per-agent swimlane or view toggle (per D3).

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
