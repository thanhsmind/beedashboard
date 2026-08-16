---
type: bee.delivery
title: homepage-terminal-full — delivery
description: "Delivery record for work item homepage-terminal-full: the homepage Terminals tab rebuilt on the same per-pane renderers the project terminal page uses."
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

Both cells capped against the CI triple: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work homepage-terminal-full` from 2 capped cell traces. Accepted at the compounding pass on 2026-08-16. Internal refactor onto shared renderers — no capped behavior_change cell, so the proposal carried empty area sections and the living specs needed no update ("the homepage's Terminals tab and the project terminal page share one shape" already stands in `docs/specs/agent-terminal.md`). No pattern candidates were proposed.
