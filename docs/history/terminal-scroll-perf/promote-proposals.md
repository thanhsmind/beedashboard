promote proposal for work item "terminal-scroll-perf" (.bee/logs/scribing-runs.jsonl + .bee/lanes/terminal-scroll-perf.json) — 1 capped cell(s): terminal-scroll-perf-1
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/terminal-scroll-perf.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/terminal-scroll-perf/delivery.md

---
type: bee.delivery
title: terminal-scroll-perf — delivery
description: "Delivery record proposed by bee knowledge promote for work item terminal-scroll-perf: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-12
bee:
  id: terminal-scroll-perf-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/terminal-scroll-perf.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/terminal-scroll-perf.json, .bee/cells/terminal-scroll-perf-1.json]
---

# terminal-scroll-perf — Delivery

## What shipped

- **terminal-scroll-perf-1** — rAF-throttled resize refit with width-gating, two-read fitScreenFont, and .term-screen scroll hints pinned by a server.rs test (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **terminal-scroll-perf-1** — `cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work terminal-scroll-perf` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/terminal-scroll-perf.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "terminal-scroll-perf" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-12T03:27:33.683Z), the work item declares no bee.areas.

area agent-terminal:
  - [terminal-scroll-perf-1] rAF-throttled resize refit with width-gating, two-read fitScreenFont, and .term-screen scroll hints pinned by a server.rs test — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/terminal-scroll-perf-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.