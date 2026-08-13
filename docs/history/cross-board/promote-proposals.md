promote proposal for work item "cross-board" (docs/history/cross-board/CONTEXT.md + docs/history/cross-board/plan.md) — 3 capped cell(s): cross-board-1, cross-board-2, cross-board-3
anchor: history — docs/history/cross-board/CONTEXT.md, docs/history/cross-board/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/cross-board/delivery.md

---
type: bee.delivery
title: cross-board — delivery
description: "Delivery record proposed by bee knowledge promote for work item cross-board: 3 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-12
bee:
  id: cross-board-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [docs/history/cross-board/CONTEXT.md, docs/history/cross-board/plan.md]
  sources: [docs/history/cross-board/CONTEXT.md, docs/history/cross-board/plan.md, .bee/cells/cross-board-1.json, .bee/cells/cross-board-2.json, .bee/cells/cross-board-3.json]
---

# cross-board — Delivery

## What shipped

- **cross-board-1** — Added read_rollup: synchronous multi-project roll-up returning per-root BeeSnapshot plus archived-feature ship times (D10) (1 file(s) changed)
- **cross-board-2** — Split bee_feature_hub_section into classify/render; added bee_cross_project_features_section merging per-project columns with D5 labels and D10/D7 Finished ordering/paging (1 file(s) changed)
- **cross-board-3** — Home page now composes a cross-project Live strip and Features board (D1) above the unchanged project list, gated on D8/D9, roll-up run off the async task via one spawn_blocking task per qualifying project (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **cross-board-1** — `cargo test --workspace. New unit tests in crates/mdview-core/src/bee.rs against scratch fixture directories: a roll-up over two roots returns one snapshot each in the order given; a feature whose archived cells all carry trace.capped_at reports the latest of them as its ship time; a feature with a mix of present and absent capped_at reports no ship time; a root with .bee/ but no archive contributes an empty archived-feature set rather than an error; a root whose archive holds an unparseable cell still yields the other features from that root. The existing no_web_framework_dependency_declared test at bee.rs:3604 stays green and unedited.`
- **cross-board-2** — `cargo test --workspace. The sixteen existing unit tests that call bee_feature_hub_section, bee_hub_finished_row, and bee_hub_finished_rows stay green -- twelve unedited, and the four that call the two renderer signatures directly updated only to pass the new arguments. The twelve feature_hub_* router tests in crates/mdview/src/server.rs stay green and unedited. New unit tests on the cross-project section: three projects with features in all three states place each feature in the same column its own project's board would and label it with its project; a merged Finished sequence orders timed entries newest first ahead of untimed entries ordered alphabetically; more than ten combined Finished entries page behind 'Show 10 more' with the remaining count taken from the merged total; the same feature slug owned by two projects renders as two rows with different labels and different links; a project contributing no features changes nothing.`
- **cross-board-3** — `cargo test --workspace. The twelve existing home_page_* router tests in crates/mdview/src/server.rs stay green and unedited, including home_page_script_selectors_match_the_markup_the_page_emits at server.rs:14339 -- the new sections are added above the existing list rather than reordering anything inside it. New router tests: a registry with several qualifying projects renders Live and Features above the project list with entries from more than one project; a registry where no project has .bee/ renders the page with neither section present; a registered root that no longer exists on disk is treated as non-qualifying rather than as an error; a project whose .bee/ holds a corrupt cell still leaves the other projects' entries on the page and the response is 200. The roll-up's blocking work is asserted structurally at the call site -- it runs inside spawn_blocking -- rather than by a timeout, because a timeout around spawn_blocking abandons the thread instead of stopping the read.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work cross-board` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/cross-board/CONTEXT.md`, `docs/history/cross-board/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "cross-board" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-12T17:14:13.260Z), the work item declares no bee.areas.

area bee-cockpit:
  - [cross-board-1] Added read_rollup: synchronous multi-project roll-up returning per-root BeeSnapshot plus archived-feature ship times (D10) — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/cross-board-1.json)
  - [cross-board-2] Split bee_feature_hub_section into classify/render; added bee_cross_project_features_section merging per-project columns with D5 labels and D10/D7 Finished ordering/paging — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/cross-board-2.json)
  - [cross-board-3] Home page now composes a cross-project Live strip and Features board (D1) above the unchanged project list, gated on D8/D9, roll-up run off the async task via one spawn_blocking task per qualifying project — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/cross-board-3.json)

area web-interface:
  - [cross-board-1] Added read_rollup: synchronous multi-project roll-up returning per-root BeeSnapshot plus archived-feature ship times (D10) — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/cross-board-1.json)
  - [cross-board-2] Split bee_feature_hub_section into classify/render; added bee_cross_project_features_section merging per-project columns with D5 labels and D10/D7 Finished ordering/paging — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/cross-board-2.json)
  - [cross-board-3] Home page now composes a cross-project Live strip and Features board (D1) above the unchanged project list, gated on D8/D9, roll-up run off the async task via one spawn_blocking task per qualifying project — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/cross-board-3.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 3 capped cell(s) mined, 1 delivery draft, 6 area bullet(s), 0 pattern candidate(s), 0 file(s) written.