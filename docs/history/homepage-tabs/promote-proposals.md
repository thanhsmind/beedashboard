promote proposal for work item "homepage-tabs" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): homepage-tabs-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/homepage-tabs/delivery.md

---
type: bee.delivery
title: homepage-tabs — delivery
description: "Delivery record proposed by bee knowledge promote for work item homepage-tabs: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: homepage-tabs-delivery
  lifecycle: active
  areas: [bee-cockpit, web-interface]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/archive/homepage-tabs/homepage-tabs-1.json]
---

# homepage-tabs — Delivery

## What shipped

- **homepage-tabs-1** — shipped (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **homepage-tabs-1** — `cargo test --workspace green. Through the router: `/` and `/?tab=kanban` serve the Features section and no Projects listing; `/?tab=projects` serves the Projects listing and no Features section; both carry the tab strip with exactly one `fg-tab--on` and one `aria-current="page"`; an unknown, empty or repeated `tab` value resolves to Kanban; a request carrying `register_error` serves the Projects tab and its banner even when `tab=kanban` is asked for; and a state where no feature qualifies serves the Projects page with no `fg-tabs` strip at all. Existing homepage assertions that expect both sections on one response are updated -- grep server.rs for tests hitting `/` before finishing.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work homepage-tabs` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "homepage-tabs" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-14T10:12:34.318Z), the work item declares no bee.areas.

area bee-cockpit:
  - [homepage-tabs-1] shipped — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/archive/homepage-tabs/homepage-tabs-1.json)

area web-interface:
  - [homepage-tabs-1] shipped — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/archive/homepage-tabs/homepage-tabs-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 2 area bullet(s), 0 pattern candidate(s), 0 file(s) written.