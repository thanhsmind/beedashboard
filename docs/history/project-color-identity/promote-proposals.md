promote proposal for work item "project-color-identity" (.bee/logs/scribing-runs.jsonl) — 4 capped cell(s): project-color-identity-1, project-color-identity-2, project-color-identity-3, project-color-identity-4
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/project-color-identity/delivery.md

---
type: bee.delivery
title: project-color-identity — delivery
description: "Delivery record proposed by bee knowledge promote for work item project-color-identity: 4 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: project-color-identity-delivery
  lifecycle: active
  areas: [bee-cockpit, appearance]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/project-color-identity-1.json, .bee/cells/project-color-identity-2.json, .bee/cells/project-color-identity-3.json, .bee/cells/project-color-identity-4.json]
---

# project-color-identity — Delivery

## What shipped

- **project-color-identity-1** — Cross-project cards now show the project name (not slug) as subtitle and a fixed per-project accent colour; per-project boards render byte-identical (1 file(s) changed)
- **project-color-identity-2** — Threaded a stable colour map into the board card and Finished row, folded worktree state into the project subtitle, and removed the card's chip row (shared renderer, so per-project boards lose it too) (2 file(s) changed)
- **project-color-identity-3** — Keep the worktree visible on a single project's own board (0 file(s) changed)
- **project-color-identity-4** — A card with no recorded title now names its worktree state in the subtitle, with no slug half and no separator, instead of dropping the line and the state with it (0 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **project-color-identity-1** — `cargo test --workspace green. New tests: the colour index is stable -- the same project id yields the same index across repeated calls in one test and the expected constant for a known id -- and two different ids that must not collide are asserted to differ. A card rendered with a project label carries the project name in the subtitle, carries no project chip, and its shell carries a `bee-hub__shell--p` modifier; the Finished row for the same project carries the matching modifier on its project span. A card rendered with no project label is asserted byte-identical to its pre-change output: slug subtitle, no modifier class, no colour. The style block is asserted to carry the five accent rules and the border rule. Existing assertions in server.rs that pin the chip row or the slug subtitle for cross-project board cards are updated -- grep for `bee-hub__slug`, `fg-chip fg-chip--neutral` and `bee-hub__row-project` across the workspace before finishing.`
- **project-color-identity-2** — `cargo test --workspace green. Tests: the colour map gives ten distinct projects ten distinct slots and gives the same project the same slot in a card and in a Finished row; a set of three real board project ids (`anphabe-gogl`, `beedashboard`, `beehive`) is asserted to receive three DIFFERENT slots -- this is the case that regressed. The project line renders all four worktree spellings (branch name, `worktree`, `merged`, `Main`) after the ` / `. No card markup contains `bee-hub__chips`, `fg-chip fg-chip--` for the group label, or the worktree chip. Existing assertions across the workspace that pin the chip row, the group chip or the worktree chip on a board card are updated -- grep for `bee-hub__chips`, `fg-chip`, `Open ·`, `Open worktree`, `Merged` and `Main` before finishing, and keep the per-project-board assertions honest.`
- **project-color-identity-3** — `cargo test --workspace green. A card rendered with `project_label: None` carries its slug, a ` / ` and the worktree spelling in the subtitle, carries no `bee-hub__shell--p` modifier and no chip row; all four worktree spellings are covered. The cross-project board's own card output is asserted unchanged by this cell.`
- **project-color-identity-4** — `cargo test --workspace green, plus a case asserting a title-less per-project card renders the worktree subtitle and does not repeat its slug.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work project-color-identity` from 4 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "project-color-identity" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-14T09:52:40.278Z), the work item declares no bee.areas.

area bee-cockpit:
  - [project-color-identity-1] Cross-project cards now show the project name (not slug) as subtitle and a fixed per-project accent colour; per-project boards render byte-identical — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/project-color-identity-1.json)
  - [project-color-identity-2] Threaded a stable colour map into the board card and Finished row, folded worktree state into the project subtitle, and removed the card's chip row (shared renderer, so per-project boards lose it too) — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/project-color-identity-2.json)
  - [project-color-identity-3] Keep the worktree visible on a single project's own board — feature-wide sync per the scribing stamp, 0 file(s) changed (trace .bee/cells/project-color-identity-3.json)
  - [project-color-identity-4] A card with no recorded title now names its worktree state in the subtitle, with no slug half and no separator, instead of dropping the line and the state with it — feature-wide sync per the scribing stamp, 0 file(s) changed (trace .bee/cells/project-color-identity-4.json)

area appearance:
  - [project-color-identity-1] Cross-project cards now show the project name (not slug) as subtitle and a fixed per-project accent colour; per-project boards render byte-identical — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/project-color-identity-1.json)
  - [project-color-identity-2] Threaded a stable colour map into the board card and Finished row, folded worktree state into the project subtitle, and removed the card's chip row (shared renderer, so per-project boards lose it too) — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/project-color-identity-2.json)
  - [project-color-identity-3] Keep the worktree visible on a single project's own board — feature-wide sync per the scribing stamp, 0 file(s) changed (trace .bee/cells/project-color-identity-3.json)
  - [project-color-identity-4] A card with no recorded title now names its worktree state in the subtitle, with no slug half and no separator, instead of dropping the line and the state with it — feature-wide sync per the scribing stamp, 0 file(s) changed (trace .bee/cells/project-color-identity-4.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 4 capped cell(s) mined, 1 delivery draft, 8 area bullet(s), 0 pattern candidate(s), 0 file(s) written.