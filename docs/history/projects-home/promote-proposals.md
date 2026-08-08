promote proposal for work item "projects-home" (.bee/logs/scribing-runs.jsonl) — 3 capped cell(s): projects-home-1, projects-home-2, projects-home-3
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/projects-home/delivery.md

---
type: bee.delivery
title: projects-home — delivery
description: "Delivery record proposed by bee knowledge promote for work item projects-home: 3 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-08
bee:
  id: projects-home-delivery
  lifecycle: active
  areas: [web-interface, agent-terminal, system-overview]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/projects-home-1.json, .bee/cells/projects-home-2.json, .bee/cells/projects-home-3.json]
---

# projects-home — Delivery

## What shipped

- **projects-home-1** — Badge every project row with its own boundary-matched terminal panes, gated on terminal.enabled with a timeout-wrapped snapshot (3 file(s) changed)
- **projects-home-2** — Added the D7/D8 add-project form and register route, with the D9a/D10 ordered validation (bounded pre-flight, deny-list, canonical-path duplicate check) plus the two carried-over badge-slice test gaps (4 file(s) changed)
- **projects-home-3** — Closed D9b deny-list containment gap, moved register validation off the async thread, restored the switch-off badge test, and split the store-failure/time-budget/duplicate-query-key edge cases into their own honest codes (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **projects-home-1** — `cargo test --workspace`
- **projects-home-2** — `cargo test --workspace`
- **projects-home-3** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work projects-home` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "projects-home" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-08T02:18:38.234Z), the work item declares no bee.areas.

area web-interface:
  - [projects-home-1] Badge every project row with its own boundary-matched terminal panes, gated on terminal.enabled with a timeout-wrapped snapshot — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/projects-home-1.json)
  - [projects-home-2] Added the D7/D8 add-project form and register route, with the D9a/D10 ordered validation (bounded pre-flight, deny-list, canonical-path duplicate check) plus the two carried-over badge-slice test gaps — feature-wide sync per the scribing stamp, 4 file(s) changed (trace .bee/cells/projects-home-2.json)
  - [projects-home-3] Closed D9b deny-list containment gap, moved register validation off the async thread, restored the switch-off badge test, and split the store-failure/time-budget/duplicate-query-key edge cases into their own honest codes — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/projects-home-3.json)

area agent-terminal:
  - [projects-home-1] Badge every project row with its own boundary-matched terminal panes, gated on terminal.enabled with a timeout-wrapped snapshot — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/projects-home-1.json)
  - [projects-home-2] Added the D7/D8 add-project form and register route, with the D9a/D10 ordered validation (bounded pre-flight, deny-list, canonical-path duplicate check) plus the two carried-over badge-slice test gaps — feature-wide sync per the scribing stamp, 4 file(s) changed (trace .bee/cells/projects-home-2.json)
  - [projects-home-3] Closed D9b deny-list containment gap, moved register validation off the async thread, restored the switch-off badge test, and split the store-failure/time-budget/duplicate-query-key edge cases into their own honest codes — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/projects-home-3.json)

area system-overview:
  - [projects-home-1] Badge every project row with its own boundary-matched terminal panes, gated on terminal.enabled with a timeout-wrapped snapshot — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/projects-home-1.json)
  - [projects-home-2] Added the D7/D8 add-project form and register route, with the D9a/D10 ordered validation (bounded pre-flight, deny-list, canonical-path duplicate check) plus the two carried-over badge-slice test gaps — feature-wide sync per the scribing stamp, 4 file(s) changed (trace .bee/cells/projects-home-2.json)
  - [projects-home-3] Closed D9b deny-list containment gap, moved register validation off the async thread, restored the switch-off badge test, and split the store-failure/time-budget/duplicate-query-key edge cases into their own honest codes — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/projects-home-3.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 3 capped cell(s) mined, 1 delivery draft, 9 area bullet(s), 0 pattern candidate(s), 0 file(s) written.