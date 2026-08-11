# Feature Hub — Context

**Feature slug:** feature-hub
**Date:** 2026-08-11
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE

## Feature Boundary

Replace the Kanban cell board on `/p/:id/_bee` with a feature-centric grouped
list (Waiting on you / In Progress / Finished) and restructure the feature
detail page into a tabbed drill-down (Activity / Todos / Sub-agents) with a
chip row, styled after anthropic.com — display-only, no write path to `.bee/`.

## Locked Decisions

| ID | Decision | Rationale |
|----|----------|-----------------------------------------------|
| D1 | Feature-centric grouped list replaces the Kanban cell board: groups Waiting on you / In Progress / Finished; card = feature name, progress n/m cells, last activity age, worktree state, status icon. | User judged the cell-centric Kanban unfit; they track per feature. |
| D2 | Feature detail = tabs **Activity** (decisions, worker outcomes from cell traces, gate stamps, test results), **Todos** (cells checklist: strikethrough done, agent badge on claimed, red mark blocked), **Sub-agents** (workers: name, tier, cells capped, live heartbeat) + chip row: project, lane, worktree + merge state, duration & cell count. | Mirrors the reference UI. |
| D3 | Style follows anthropic.com: warm cream bg (#FAF9F5 / #F0EEE6 panels), near-black ink, book-cloth coral accent (#CC785C), soft rounded cards, generous spacing; dark theme keeps the warm hue family; existing theme toggle respected. | Readability, user's explicit reference. |
| D4 | Waiting-on-you group only for features with live work (open/claimed cells, or the active feature) at an unapproved gate, or a paused handoff. Finished/stale lanes never emit waiting entries. One-time data cleanup stamps the 6 stale lanes terminal. | Kills the 6 ghost gate cards permanently. |
| D5 | Display-only stays: no writes to `.bee/`, read-never-writes guarantee tests preserved (agent-board D1 / bee-board-pm D4 lineage). | — |

Binding priors: bee-board-pm D1 (English labels), D6 (attention list),
D9 (no absolute paths); archive-visibility (archived cells power Finished
group and detail pages of closed features); agent-board deviation record
(one waiting card per feature at its current stop).

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Waiting on you | A feature whose live work stands at an unapproved gate, or has a paused handoff — the user's action unblocks it. |
| Finished | A closed feature: cells archived, or lane at a terminal phase; shows merged-vs-unmerged worktree state. |

## Existing Code Context

### Reusable Assets

- `crates/mdview/src/views.rs` — `bee_agent_board_section` (being replaced), `bee_agent_card_badge`, feature/cell detail pages, `bee_board_page` section order.
- `crates/mdview-core/src/bee.rs` — snapshot readers: buckets, lanes∪active (`compute_phase_board`), `running_workers`, worktree grants, handoff pause rule, decisions reader, archived-cells reader (archive-visibility).
- `.bee/runtime/worktree-grants.json` — worktree ↔ feature ↔ branch ↔ merge state.

### Integration Points

- `crates/mdview/src/server.rs` — `bee_board()` handler, feature/cell detail handlers, test suite incl. `*_read_never_writes_the_fixtures_bee_tree`.
- CSS: inline styles in views.rs (+ `crates/mdview/assets/` if present) — palette tokens for D3.

## Outstanding Questions

### Deferred To Planning

- [ ] Activity tab timeline sources: which of decisions.jsonl / cell traces / gate stamps are cheaply joinable per feature in the existing readers.
- [ ] Whether Finished group paginates/collapses beyond a recent cap.

## Deferred Ideas

- Kanban as a secondary toggle view (user chose replacement; not planned).
- Conversation transcript view per worker (reference UI's "Open conversation") — bee stores no transcripts in `.bee/`; out of scope.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning cites,
never reinterprets.
