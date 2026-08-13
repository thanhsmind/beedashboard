promote proposal for work item "board-drop-live" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): drop-live-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/board-drop-live/delivery.md

---
type: bee.delivery
title: board-drop-live — delivery
description: "Delivery record proposed by bee knowledge promote for work item board-drop-live: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: board-drop-live-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/drop-live-1.json]
---

# board-drop-live — Delivery

## What shipped

- **drop-live-1** — Removed the cross-project Live section (function, callers, tests) while leaving Features and the per-project board's own Live strip unchanged (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **drop-live-1** — `cargo test --workspace. Two cross-board-3 router tests are updated to the new shape and nothing else changes: home_page_renders_cross_project_live_and_features_above_the_project_list_from_several_projects (crates/mdview/src/server.rs:14481) is renamed and rewritten to assert the Features section renders above the project list with entries from more than one project AND that no Live section is emitted; home_page_omits_cross_project_sections_when_no_project_qualifies (server.rs:14523) keeps asserting the page matches its plain project-list markup. The twelve home_page_* router tests stay green and unedited, and board_live_strip_renders_above_the_feature_hub_with_every_row_kind (server.rs:6422) stays green and unedited -- it covers the per-project strip, which this cell must not affect.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work board-drop-live` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "board-drop-live" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-13T03:38:32.639Z), the work item declares no bee.areas.

area bee-cockpit:
  - [drop-live-1] Removed the cross-project Live section (function, callers, tests) while leaving Features and the per-project board's own Live strip unchanged — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/drop-live-1.json)

area web-interface:
  - [drop-live-1] Removed the cross-project Live section (function, callers, tests) while leaving Features and the per-project board's own Live strip unchanged — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/drop-live-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 2 area bullet(s), 0 pattern candidate(s), 0 file(s) written.