promote proposal for work item "card-collapse-inprogress" (docs/history/card-collapse-inprogress/CONTEXT.md) — 2 capped cell(s): card-collapse-inprogress-1, card-collapse-inprogress-2
anchor: history — docs/history/card-collapse-inprogress/CONTEXT.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/card-collapse-inprogress/delivery.md

---
type: bee.delivery
title: card-collapse-inprogress — delivery
description: "Delivery record proposed by bee knowledge promote for work item card-collapse-inprogress: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: card-collapse-inprogress-delivery
  lifecycle: active
  required_context: [docs/history/card-collapse-inprogress/CONTEXT.md]
  sources: [docs/history/card-collapse-inprogress/CONTEXT.md, .bee/cells/card-collapse-inprogress-1.json, .bee/cells/card-collapse-inprogress-2.json]
---

# card-collapse-inprogress — Delivery

## What shipped

- **card-collapse-inprogress-1** — Collapsed the In Progress card behind a native details/summary header, moving its detail body and Feature detail link off the old whole-card anchor (2 file(s) changed)
- **card-collapse-inprogress-2** — Added the two missing unit tests proving bee_hub_card ships with no open attribute and its body opens with the Feature detail link's own href (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **card-collapse-inprogress-1** — `cargo test --workspace green. The seven bee_hub_card unit tests (views.rs:7916, 7992, 8064, 8098, 8128, 8165, 8189) and every server.rs page-body assertion that reads the old anchor shape (notably the `class="bee-hub__card" data-hub-group="in-progress" href=...` matches around server.rs:4776 and its siblings) are updated to the new markup, each still asserting the same property it asserts today -- the subtitle spellings, the badge shape, the empty-pane case, the shell modifier -- never deleted or loosened. Add cases for: a card renders with no ` open` attribute (collapsed by default), the body carries a `bee-hub__detail-link` pointing at the same /p/{id}/_bee/feature/{feature} href the anchor used to carry, and the badge nav sits outside the `</details>` while the shell still closes after it.`
- **card-collapse-inprogress-2** — `cargo test --workspace green with the two new tests present and passing. Sanity-check each one actually bites: temporarily emitting an `open` attribute must fail the first test, and removing the detail-link row must fail the second -- verify by reasoning against the renderer string at views.rs:3388, do not commit any such temporary edit.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work card-collapse-inprogress` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/card-collapse-inprogress/CONTEXT.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.