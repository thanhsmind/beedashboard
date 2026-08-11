---
artifact_contract: bee-plan/v1
mode: standard
---

# Plan: Feature Hub

Mode: `standard` — 2 risk flags: covered-contract-change, proof-weakening
(replaces the tested agent-board section and feature detail page; their tests
are retired and replaced in the same cells).
Why this is the least workflow that protects the work: a view-layer + reader
pivot over data the snapshot already carries, but it swaps two tested
surfaces, so shape and replacement proof gate before edits.

## Requirements (from CONTEXT.md)

D1 grouped feature list replaces Kanban; D2 tabbed detail + chip row;
D3 anthropic.com palette both themes; D4 waiting-on-you = live work only +
stale-lane cleanup; D5 display-only preserved. Priors: English labels, no
absolute paths, attention list untouched, archived cells feed Finished.

## Discovery

All data needed exists in current readers: buckets + per-feature cells
(`bee.rs` cells reader), lanes∪active (`compute_phase_board`), worktree
grants file, handoff pause rule, decisions reader, archived-cells reader
(archive-visibility), `running_workers` join. Activity tab joins per-feature:
decision events (feature-scoped), cell trace fields (worker, outcome,
capped_at, verify state), gate stamps from lane records. No new store files.

## Approach

Two serialized cells (same files), walking skeleton first; a store data
cleanup runs CLI-side by the orchestrator, not in a cell. Rejected: keeping
Kanban behind a toggle (user chose replacement); reading git for merge state
(worktree-grants.json already records it).

Groups (D1/D4): Waiting on you = live-work features at an unapproved gate or
with paused handoff; In Progress = features with open/claimed cells;
Finished = terminal-phase lanes and archive-only features, with worktree
merged/unmerged chip. A feature appears in exactly one group (waiting wins
over in-progress; finished only when no live cells).

## Shape

1. **fh-1 — skeleton + palette**: anthropic-style tokens (cream bg, ink,
   coral accent, warm dark variant) applied to the bee page; grouped feature
   list replaces `bee_agent_board_section`; cards per D1; groups per D4;
   replace agent-board section tests with hub tests incl.
   read-never-writes and no-ghost-waiting cases.
2. **fh-2 — detail tabs + chips**: feature detail restructured per D2
   (Activity / Todos / Sub-agents tabs, chip row incl. worktree + merge
   state); archived features fully populated (archive-visibility lineage);
   tests per tab + Closed/merged states.

Cleanup (orchestrator, after merge): stamp the 6 stale lanes terminal so
old records stop implying pending gates.

## Test matrix

- Happy: fixture with one feature per group renders each card in its group
  with progress n/m, age, worktree state; detail tabs show decisions/outcomes,
  cells checklist with badges, workers list; chips render lane + merge state.
- Edge: empty store → honest empty groups; archive-only feature lands in
  Finished with Closed detail; stale lane without live cells emits NO waiting
  entry (ghost-card regression test); unmerged-but-finished shows unmerged chip.
- Error: read_errors still render the page; no absolute paths; theme toggle
  renders both palettes.
- Guarantee: read-never-writes tests preserved for board + detail pages.

## Out of scope

Kanban toggle view; conversation transcripts; any write path.
