promote proposal for work item "homepage-terminal-full" (docs/history/homepage-terminal-full/CONTEXT.md + docs/history/homepage-terminal-full/plan.md) — 2 capped cell(s): homepage-terminal-full-1, homepage-terminal-full-2
anchor: history — docs/history/homepage-terminal-full/CONTEXT.md, docs/history/homepage-terminal-full/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/homepage-terminal-full/delivery.md

---
type: bee.delivery
title: homepage-terminal-full — delivery
description: "Delivery record proposed by bee knowledge promote for work item homepage-terminal-full: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: homepage-terminal-full-delivery
  lifecycle: active
  areas: [web-interface, agent-terminal]
  required_context: [docs/history/homepage-terminal-full/CONTEXT.md, docs/history/homepage-terminal-full/plan.md]
  sources: [docs/history/homepage-terminal-full/CONTEXT.md, docs/history/homepage-terminal-full/plan.md, .bee/cells/homepage-terminal-full-1.json, .bee/cells/homepage-terminal-full-2.json]
---

# homepage-terminal-full — Delivery

## What shipped

- **homepage-terminal-full-1** — Renderers take a per-pane link and base (2 file(s) changed)
- **homepage-terminal-full-2** — Rebuild the Terminals tab on the shared renderers (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **homepage-terminal-full-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- **homepage-terminal-full-2** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work homepage-terminal-full` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/homepage-terminal-full/CONTEXT.md`, `docs/history/homepage-terminal-full/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "homepage-terminal-full" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-15T14:25:30.783Z), the work item declares no bee.areas.

area web-interface:
  (no capped behavior_change cell exists for this feature)

area agent-terminal:
  (no capped behavior_change cell exists for this feature)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.