promote proposal for work item "card-badge-inside" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): card-badge-inside-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/card-badge-inside/delivery.md

---
type: bee.delivery
title: card-badge-inside — delivery
description: "Delivery record proposed by bee knowledge promote for work item card-badge-inside: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-14
bee:
  id: card-badge-inside-delivery
  lifecycle: active
  areas: [bee-cockpit, appearance]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/card-badge-inside-1.json]
---

# card-badge-inside — Delivery

## What shipped

- **card-badge-inside-1** — Draw the terminal badges inside the feature card's own box (0 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **card-badge-inside-1** — `cargo test --workspace green. The existing shape test bee_hub_card_emits_terminal_badges_matching_project_badges_markup_shape (views.rs around line 6684) still holds -- the nav is still a sibling after the card's own `</a>` -- and gains assertions that the pair is wrapped by a `fg-card bee-hub__shell` div, that the anchor itself no longer carries `fg-card`, and that the nav closes before the shell's closing tag. The server-side assertion at server.rs line 4542 that pins the literal `class="fg-card bee-hub__card" data-hub-group=` string is updated to the new class list. Add a case that a card with no panes renders the shell and the anchor but no nav.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work card-badge-inside` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "card-badge-inside" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-14T07:45:16.708Z), the work item declares no bee.areas.

area bee-cockpit:
  - [card-badge-inside-1] Draw the terminal badges inside the feature card's own box — feature-wide sync per the scribing stamp, 0 file(s) changed (trace .bee/cells/card-badge-inside-1.json)

area appearance:
  - [card-badge-inside-1] Draw the terminal badges inside the feature card's own box — feature-wide sync per the scribing stamp, 0 file(s) changed (trace .bee/cells/card-badge-inside-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 2 area bullet(s), 0 pattern candidate(s), 0 file(s) written.