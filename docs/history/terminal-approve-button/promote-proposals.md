promote proposal for work item "terminal-approve-button" (docs/history/terminal-approve-button/CONTEXT.md) — 1 capped cell(s): terminal-approve-button-1
anchor: history — docs/history/terminal-approve-button/CONTEXT.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/terminal-approve-button/delivery.md

---
type: bee.delivery
title: terminal-approve-button — delivery
description: "Delivery record proposed by bee knowledge promote for work item terminal-approve-button: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: terminal-approve-button-delivery
  lifecycle: active
  areas: [web-interface, agent-terminal]
  required_context: [docs/history/terminal-approve-button/CONTEXT.md]
  sources: [docs/history/terminal-approve-button/CONTEXT.md, .bee/cells/archive/terminal-approve-button/terminal-approve-button-1.json]
---

# terminal-approve-button — Delivery

## What shipped

- **terminal-approve-button-1** — Added a one-tap Approve button beside Stage, wired in app.js and UNASSIGNED_TERMINAL_SCRIPT (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **terminal-approve-button-1** — `cargo test --workspace green. New cases: pane_controls renders the Approve button, with its markup positioned BEFORE the Stage button inside term-reply__actions (assert on relative index, not just presence); the shared button CSS rule names term-reply__approve alongside its two siblings; app.js contains an Approve handler posting the exact text "Approve" with submit true; UNASSIGNED_TERMINAL_SCRIPT contains the same handler for the unassigned input URL (assert against the constant the way existing tests assert on that script's contents). Existing tests that name .term-reply__send/.term-reply__stage (views.rs:6316-6344) keep asserting what they assert today.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work terminal-approve-button` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/terminal-approve-button/CONTEXT.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "terminal-approve-button" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-15T14:25:30.802Z), the work item declares no bee.areas.

area web-interface:
  (no capped behavior_change cell exists for this feature)

area agent-terminal:
  (no capped behavior_change cell exists for this feature)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.