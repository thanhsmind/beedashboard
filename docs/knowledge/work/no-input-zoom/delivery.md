---
type: bee.delivery
title: no-input-zoom — delivery
description: "Delivery record for work item no-input-zoom: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: no-input-zoom-delivery
  lifecycle: active
  areas: [appearance, agent-terminal, settings]
  required_context: [docs/specs/appearance.md, docs/specs/agent-terminal.md, docs/specs/settings.md]
  sources: [.bee/cells/archive/no-input-zoom/no-input-zoom-1.json]
---

# no-input-zoom — Delivery

## What shipped

Tapping into a text field no longer magnifies the page on a touch device. A
touch browser zooms the whole layout whenever it focuses a control whose text
is smaller than sixteen pixels, and the zoom stays after the field is left — so
one tap into the terminal reply box left the interface blown up until the
reader pinched it back.

Every field the reader can focus — the terminal reply box and the settings
inputs and selects — now renders its own text at sixteen pixels when the
pointer is coarse, which is the size below which the zoom fires. Pointing
devices that are not coarse keep the compact sizes the interface was designed
at: thirteen pixels for the reply box, fourteen for the settings fields.

The page's own zoom controls were deliberately left alone. Declaring the page
unscalable would also stop the zoom, and would take pinch-zoom away from every
reader who relies on it; the size rule fixes the accidental zoom without
removing the deliberate one.

## Verify

`cargo test --workspace` green. Two new assertions guard the two halves
separately — the shared stylesheet carries the coarse-pointer rule for the
settings fields, and the terminal view's own inline style block carries it for
the reply box — because the terminal's block is applied after the shared
stylesheet and would otherwise win at equal specificity, so a single rule in
one place would not have held.

## Deviations

None recorded in the capped cell trace.

## Provenance

Written at feature close from the capped cell trace of `no-input-zoom-1`. The
sixteen-pixel threshold and the choice of a size rule over a page-level zoom
lock are both recorded in the decision log.
