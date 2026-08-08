promote proposal for work item "terminal-image-attach" (docs/history/terminal-image-attach/CONTEXT.md + docs/history/terminal-image-attach/plan.md) — 3 capped cell(s): terminal-image-attach-1, terminal-image-attach-2, terminal-image-attach-3
anchor: history — docs/history/terminal-image-attach/CONTEXT.md, docs/history/terminal-image-attach/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/terminal-image-attach/delivery.md

---
type: bee.delivery
title: terminal-image-attach — delivery
description: "Delivery record proposed by bee knowledge promote for work item terminal-image-attach: 3 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-08
bee:
  id: terminal-image-attach-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [docs/history/terminal-image-attach/CONTEXT.md, docs/history/terminal-image-attach/plan.md]
  sources: [docs/history/terminal-image-attach/CONTEXT.md, docs/history/terminal-image-attach/plan.md, .bee/cells/terminal-image-attach-1.json, .bee/cells/terminal-image-attach-2.json, .bee/cells/terminal-image-attach-3.json]
---

# terminal-image-attach — Delivery

## What shipped

- **terminal-image-attach-1** — Added POST /p/:id/_terminal/:pane_id/attach with switch/boundary guards, MIME allowlist + magic-byte sniffing, 10MB cap, 32-file pane cap, sanitized rand-named storage, and 10 route tests (1 file(s) changed)
- **terminal-image-attach-2** — Wired composer attach UI (picker, drag-drop, paste, chips, one-message send) gated to project terminal pages, with a render test proving Unassigned renders none of it (3 file(s) changed)
- **terminal-image-attach-3** — Prune stale attach files before the 32-file cap, quote whitespace paths in composeMessage, and pre-check upload size client-side (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **terminal-image-attach-1** — `cargo test -p mdview terminal_attach -- --nocapture reports the new attach tests green; cargo test --workspace stays green at cap`
- **terminal-image-attach-2** — `cargo test -p mdview terminal_page -- --nocapture shows the attach render assertions green (project page yes, unassigned no); cargo test --workspace stays green at cap`
- **terminal-image-attach-3** — `cargo test -p mdview terminal_attach -- --nocapture shows the prune/cap tests green; cargo test --workspace stays green at cap`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work terminal-image-attach` from 3 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/terminal-image-attach/CONTEXT.md`, `docs/history/terminal-image-attach/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "terminal-image-attach" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-08T12:25:51.121Z), the work item declares no bee.areas.

area agent-terminal:
  - [terminal-image-attach-1] Added POST /p/:id/_terminal/:pane_id/attach with switch/boundary guards, MIME allowlist + magic-byte sniffing, 10MB cap, 32-file pane cap, sanitized rand-named storage, and 10 route tests — feature-wide sync per the scribing stamp, 1 file(s) changed (trace .bee/cells/terminal-image-attach-1.json)
  - [terminal-image-attach-2] Wired composer attach UI (picker, drag-drop, paste, chips, one-message send) gated to project terminal pages, with a render test proving Unassigned renders none of it — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/terminal-image-attach-2.json)
  - [terminal-image-attach-3] Prune stale attach files before the 32-file cap, quote whitespace paths in composeMessage, and pre-check upload size client-side — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/terminal-image-attach-3.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 3 capped cell(s) mined, 1 delivery draft, 3 area bullet(s), 0 pattern candidate(s), 0 file(s) written.