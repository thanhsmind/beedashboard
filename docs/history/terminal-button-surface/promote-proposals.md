promote proposal for work item "terminal-button-surface" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): terminal-button-surface-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/terminal-button-surface/delivery.md

---
type: bee.delivery
title: terminal-button-surface — delivery
description: "Delivery record proposed by bee knowledge promote for work item terminal-button-surface: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-08
bee:
  id: terminal-button-surface-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/terminal-button-surface-1.json]
---

# terminal-button-surface — Delivery

## What shipped

- **terminal-button-surface-1** — Terminal buttons use a defined surface token and the arrows share one row with the named keys (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **terminal-button-surface-1** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work terminal-button-surface` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "terminal-button-surface" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-08T02:45:36.080Z), the work item declares no bee.areas.

area agent-terminal:
  - [terminal-button-surface-1] Terminal buttons use a defined surface token and the arrows share one row with the named keys — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/terminal-button-surface-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.