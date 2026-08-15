# Kanban Live Signals — Context

**Feature slug:** kanban-live-signals
**Date:** 2026-08-15
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE | READ

## Feature Boundary

Kanban cards (home Kanban tab and per-project bee board) surface the live bee
signals the dashboard already has on disk but never reads — activity, run
state, and deferred debt — entirely at card level; no page panel is added or
restored, and no board column changes.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | Card "Last activity" reads `state.json` `last_activity` as the primary timestamp, and tails `.bee/logs/tools.jsonl` for a live "working now" indicator (pulse) when a tool call landed within ~2 minutes. Cell `claimed_at`/`capped_at` stops being the only source. | `last_activity` updates on every tool call — work without a claimed cell no longer renders as a frozen card. Decision log: 112b48dd. |
| D2 | `run_state` (`shaping` / `awaiting-approval` / `running` / `blocked` / `done`) from `state.json` renders as a colored badge on the card; `awaiting-approval` is visually prominent. Column placement unchanged. | Makes the pre-cell shape/gate phase visible. Decision log: a30a1465. |
| D3 | Deferred-queue debt (unresolved `.bee/deferred-queue.jsonl` entries) shows as a count badge on the card with detail on hover/click; no dedicated panel. | Decision log: 1e937e51. |
| D4 | board-trim stands: Sessions and Process-health panels stay removed. All new signals land on cards; parsed-but-unrendered readers (reservations, `tier_mix`, findings) stay dormant. | User reaffirmed the trim 2026-08-15. Decision log: 8a7a6a63. |

### Agent's Discretion

- Exact badge colors, pulse styling, and hover/click detail markup — must fit
  the board's existing visual language.
- The precise liveness window (~2 minutes) may be tuned; order of magnitude is
  locked.
- How `tools.jsonl` is read efficiently (tail window, byte cap) — file is
  ~1.4 MB and append-only; reading it whole per request is not acceptable.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| working now | At least one `tools.jsonl` entry with `ts` within the liveness window (~2 min) at render time |
| deferred debt | `deferred-queue.jsonl` entries whose lifecycle events leave them unresolved (an `add` without a matching resolve/flush event) |

## Existing Code Context

From the verification scout. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance-core/src/bee.rs:141-172` — `BeeState` deserializer;
  add `last_activity` and `run_state` fields here (file already read per snapshot).
- `crates/waggledance/src/views.rs:3736-3749` — `bee_hub_latest_activity`
  (current claimed_at/capped_at max); D1 changes its inputs.
- `crates/waggledance/src/views.rs:2532-2536, 2651-2659` — `BeeHubCardData`
  assembly; `views.rs:3381-3480` — `bee_hub_card` render.

### Established Patterns

- `bee.rs:1-75` module doc enumerates every `.bee` file read (D9 there says
  `.bee/logs/` never opened) — that doc contract must be updated alongside the
  new readers.
- board-liveness live strip (`views.rs:2274`) — existing precedent for
  liveness-window filtering on sessions.

### Integration Points

- `crates/waggledance-core/src/bee.rs` — new readers: `state.json` extra
  fields, `tools.jsonl` tail, `deferred-queue.jsonl` replay.
- `crates/waggledance/src/views.rs` — card badge/pulse/debt rendering + CSS.

## Canonical References

- `docs/specs/bee-cockpit.md` — board spec; update at scribing.
- Decision «Kanban board pages drop the 1200px clamp…» (2026-08-15) — current
  board layout baseline; do not disturb.

## Outstanding Questions

### Deferred To Planning

- [ ] `deferred-queue.jsonl` resolution semantics — inspect event kinds beyond
  `add` (flush/resolve) to define "unresolved" precisely.
- [ ] `tools.jsonl` tail strategy — seek-from-end byte window vs line cap;
  measure against the 1.4 MB fixture.
- [ ] Cross-project boards: confirm each project card reads its own project's
  `.bee` files through the existing snapshot path.

## Deferred Ideas

- Rendering reservations / tier-mix / findings anywhere — readers stay dormant
  per D4; revisit only on explicit request.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
