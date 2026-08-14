promote proposal for work item "homepage-terminals" (docs/history/homepage-terminals/CONTEXT.md + docs/history/homepage-terminals/plan.md) — 2 capped cell(s): homepage-terminals-1, homepage-terminals-2
anchor: history — docs/history/homepage-terminals/CONTEXT.md, docs/history/homepage-terminals/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/homepage-terminals/delivery.md

---
type: bee.delivery
title: homepage-terminals — delivery
description: "Delivery record proposed by bee knowledge promote for work item homepage-terminals: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: homepage-terminals-delivery
  lifecycle: active
  required_context: [docs/history/homepage-terminals/CONTEXT.md, docs/history/homepage-terminals/plan.md]
  sources: [docs/history/homepage-terminals/CONTEXT.md, docs/history/homepage-terminals/plan.md, .bee/cells/homepage-terminals-1.json, .bee/cells/homepage-terminals-2.json]
---

# homepage-terminals — Delivery

## What shipped

- **homepage-terminals-1** — Added the read-only Terminals tab to the homepage (HomeTab::Terminals, terminals_tab, terminals_menu_panes, data-term-base poller branch); D1-D5, D7, D8 covered; shipped in 1906813. (3 file(s) changed)
- **homepage-terminals-2** — Let the homepage Terminals tab type into the selected agent (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **homepage-terminals-1** — `cargo test --workspace`
- **homepage-terminals-2** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work homepage-terminals` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/homepage-terminals/CONTEXT.md`, `docs/history/homepage-terminals/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.