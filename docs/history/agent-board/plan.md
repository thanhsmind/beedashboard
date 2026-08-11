---
artifact_contract: bee-plan/v1
mode: standard
# approved_gate2: <unset until approval; then a date stamp — the only permitted post-approval write>
---

# Plan: Agent Board (Kanban)

Mode: `standard` — 2 risk flags: covered-contract-change, proof-weakening
(replaces the tested phase-board output and retires its tests).
Why this is the least workflow that protects the work: one visible surface
swap backed by an existing snapshot — no new data readers, no write paths —
but it deletes six passing tests, so the shape and the replacement proof are
reviewed before any edit.

## Requirements (from CONTEXT.md)

- D1: display-only; never writes `.bee/`; the `*_read_never_writes_the_fixtures_bee_tree` guarantee stays tested.
- D2: card unit = cell; feature name is card metadata.
- D3: agent badge on each card; status columns, no swimlanes.
- D4: "Review" column = waiting on the user (pending gates, paused handoffs) — never an automatic independent-review queue (bee-board-pm D7 preserved).
- D5: Kanban board replaces `bee_phase_board_section` as the main board on `/p/:id/_bee`; data readers stay.

Binding priors: bee-board-pm D1 (English labels), D3 (cards link to detail
pages, no drawers), D5 (fixed section order, honest empty states), D6
(attention list unchanged), D9 (no absolute paths).

## Discovery

Inspected `crates/mdview-core/src/bee.rs` readers and `crates/mdview/src/views.rs`
board code (bee-gather digest, file:line anchors in CONTEXT.md). Finding: the
snapshot already carries everything the five columns need — no
`mdview-core` change required. Cell statuses are verbatim `open / claimed /
blocked / capped / dropped` (bee.rs:93-96); D7 buckets map claimed→Doing,
open→Waiting, blocked→Stuck, capped→Done (bee.rs:941-954). Agent identity =
`BeeCell.worker` plus live join `running_workers` (nickname ↔ session id,
bee.rs:1062-1089). PBIs carry `proposed / in-flight / parked / done /
declined` (bee.rs:388-390). Waiting-on-user signals: `approved_gates` on
state and lanes, handoff kind pause (bee.rs:1288-1301). No per-cell "in
review" status exists — confirms D4's framing.

## Approach

Recommended: pure view-layer swap (per D5) in `crates/mdview/src/views.rs` —
new `bee_agent_board_section()` renders five columns from existing snapshot
fields; `bee_phase_board_section`/`bee_phase_card`/`LIFECYCLE_ORDER` retire;
tests in `crates/mdview/src/server.rs` replaced one-for-one plus new cases.

Column mapping (resolves CONTEXT.md deferred question 1):

| Column | Source | Card shape |
|---|---|---|
| Backlog | `backlog.pbis` where status `proposed` or `parked` (parked marked) | PBI card (visually lighter) |
| Todo | cells `open` (`buckets.waiting`) | cell card |
| In Progress | cells `claimed` (`buckets.doing`) + cells `blocked` with a blocked marker | cell card + agent badge |
| Review | decision cards: each unapproved gate of the active feature and each lane at its current stop, plus a paused handoff | decision card ("waiting on you") |
| Done | cells `capped` (`buckets.done`), collapsed beyond a recent cap (reuse `<details>` pattern) | cell card |

Stuck cells (resolves deferred question 2): blocked cells stay in
**In Progress** with a visible blocked marker; the D6 attention list keeps
its Critical blocked-cells entry unchanged — Kanban never replaces attention.

Agent badge: `cell.worker`; if a `running_workers` row matches the cell, badge
renders live (heartbeat); otherwise plain name; absent worker → no badge.
`dropped`/unrecognized statuses keep current behavior: not rendered (D7 parity).

Rejected alternatives:
- New `mdview-core` compute for the board — snapshot already sufficient; would widen the diff for nothing.
- Cell-shaped Review column — no such cell status exists; would fabricate state (violates honest-data rule).
- Keeping both boards side by side — user chose replacement (D5).

Risk map: views.rs section swap MEDIUM (six tests retire — proof replaced in
the same cell, never a window with less coverage); server.rs tests LOW
(pattern exists); mdview-core NONE (untouched).

## Shape

One slice, two serialized cells (both own `views.rs` + `server.rs` tests;
walking skeleton first):

1. **ab-1 — skeleton**: replace phase-board section with the Kanban board
   rendering Todo / In Progress / Done from buckets, agent badges, blocked
   markers, honest empty states; replace the six phase-board tests with
   Kanban equivalents incl. `agent_board_read_never_writes_the_fixtures_bee_tree`.
2. **ab-2 — Backlog + Review columns**: PBI cards and waiting-on-you decision
   cards (gates, paused handoff), Done-column collapse, links to detail
   pages; tests for both columns and empty states.

## Test matrix

Triad, smallest demonstrating size, in `server.rs` integration tests over
fixtures (existing pattern):
- Happy: fixture with open/claimed/blocked/capped cells + a proposed PBI + an unapproved gate renders each in its column; badge shows worker nickname; live worker badge marked.
- Edge: empty store → five honest empty columns; stale (non-live) session → plain badge; capped overflow collapses; parked PBI marked.
- Error: `read_errors` present → board still renders; unrecognized cell status → ignored without panic; no absolute path appears in output (D9).
- Guarantee: `agent_board_read_never_writes_the_fixtures_bee_tree` (byte-identical `.bee/` tree).

## Out of scope

- Any write path, Approve/Reject buttons (deferred per D1; backlog row exists).
- Per-agent swimlanes/toggle (backlog row exists).
- mdview-core reader changes, attention-list changes, KPI tiles, stepper.
