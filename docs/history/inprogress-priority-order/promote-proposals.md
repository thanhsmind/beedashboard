promote proposal for work item "inprogress-priority-order" (docs/history/inprogress-priority-order/CONTEXT.md + docs/history/inprogress-priority-order/plan.md) — 2 capped cell(s): inprogress-priority-order-1, inprogress-priority-order-2
anchor: history — docs/history/inprogress-priority-order/CONTEXT.md, docs/history/inprogress-priority-order/plan.md
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/inprogress-priority-order/delivery.md

---
type: bee.delivery
title: inprogress-priority-order — delivery
description: "Delivery record proposed by bee knowledge promote for work item inprogress-priority-order: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-15
bee:
  id: inprogress-priority-order-delivery
  lifecycle: active
  required_context: [docs/history/inprogress-priority-order/CONTEXT.md, docs/history/inprogress-priority-order/plan.md]
  sources: [docs/history/inprogress-priority-order/CONTEXT.md, docs/history/inprogress-priority-order/plan.md, .bee/cells/inprogress-priority-order-1.json, .bee/cells/inprogress-priority-order-2.json]
---

# inprogress-priority-order — Delivery

## What shipped

- **inprogress-priority-order-1** — Threaded real terminal pane data into the per-project bee board's In Progress cards via the existing project_feature_panes join (2 file(s) changed)
- **inprogress-priority-order-2** — Sorted In Progress cards by a shared D7 blocked>working>rest comparator on both boards, merged the homepage column flat across projects, added the blocked-pane reason line, and moved In Progress first on mobile (1 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **inprogress-priority-order-1** — `cargo test --workspace green. New cases: the per-project bee board renders a terminal badge nav for an In Progress feature whose checkout has panes, and renders none when the herdr snapshot is absent; the not-found path for a project with no .bee/ store still returns not found. Existing bee_board / bee_board_page / bee_feature_hub_section tests keep asserting what they assert today -- update call sites for the new argument, never delete or loosen an assertion.`
- **inprogress-priority-order-2** — `cargo test --workspace green. New cases: three cards -- one with a blocked pane and the OLDEST activity, one with a working pane, one with neither and the NEWEST activity -- render blocked, working, neither, proving tiers beat activity; a pane with status idle/done/unknown/shell earns no tier; within one tier newer activity wins, a None-activity card renders last, and two otherwise-identical cards fall back to name order; the homepage In Progress column interleaves two projects' cards by the comparator instead of grouping them, while a dense-row column on the same render keeps its per-project grouping; a card with a blocked pane carries the exact `Waiting on you — a terminal is blocked` line, the same card with a gate reason carries BOTH lines with the gate line first, and a card with no blocked pane carries neither; the max-width:700px block carries the In Progress order rule and no ordering rule appears outside that media query.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work inprogress-priority-order` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/inprogress-priority-order/CONTEXT.md`, `docs/history/inprogress-priority-order/plan.md`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

None: the work item declares no bee.areas, so there is no area to sync (D19).

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 0 area bullet(s), 0 pattern candidate(s), 0 file(s) written.