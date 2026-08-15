promote proposal for work item "home-terminal-parity" (docs/history/home-terminal-parity/plan.md) — 2 capped cell(s): home-terminal-parity-1, home-terminal-parity-2
anchor: history — docs/history/home-terminal-parity/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/home-terminal-parity/delivery.md

---
type: bee.delivery
title: home-terminal-parity — delivery
description: "Delivery record proposed by bee knowledge promote for work item home-terminal-parity: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: home-terminal-parity-delivery
  lifecycle: active
  required_context: [docs/history/home-terminal-parity/plan.md]
  sources: [docs/history/home-terminal-parity/plan.md, .bee/cells/home-terminal-parity-1.json, .bee/cells/home-terminal-parity-2.json]
---

# home-terminal-parity — Delivery

## What shipped

- **home-terminal-parity-1** — Homepage terminal screen gains scroll history stack and pane identity line, mirroring pane_cards; app.js scroll handlers honour the pane's own data-term-base (3 file(s) changed)
- **home-terminal-parity-2** — Replaced the homepage Terminals tab's select switcher with the homepage-mode Agents drawer, added project-scoped create controls, and updated/added the affected server.rs tests (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **home-terminal-parity-1** — `cargo test --workspace`
- **home-terminal-parity-2** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work home-terminal-parity` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/home-terminal-parity/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.