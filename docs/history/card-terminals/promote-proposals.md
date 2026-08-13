promote proposal for work item "card-terminals" (.bee/logs/scribing-runs.jsonl) — 1 capped cell(s): card-terminals-1
anchor: ledger — .bee/logs/scribing-runs.jsonl
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/card-terminals/delivery.md

---
type: bee.delivery
title: card-terminals — delivery
description: "Delivery record proposed by bee knowledge promote for work item card-terminals: 1 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-13
bee:
  id: card-terminals-delivery
  lifecycle: active
  areas: [bee-cockpit]
  required_context: [.bee/logs/scribing-runs.jsonl]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/cells/card-terminals-1.json]
---

# card-terminals — Delivery

## What shipped

- **card-terminals-1** — Cards on the cross-project Features board badge the terminal panes running in that feature's own checkout, joined on the checkout directory in server.rs and rendered by views.rs (2 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **card-terminals-1** — `cargo test --workspace. New router tests on the home page: a pane whose cwd sits in a feature's own worktree directory renders on that feature's card and on no other; a pane in the project's main checkout renders on every card of that project that has no worktree, and never on a card that has one; a feature with no pane at all renders no badge container; with the terminal switch off no card carries a badge and the page otherwise matches what it renders now; the Finished rows carry no badge. A new views unit test asserts bee_hub_card emits the same badge markup shape project_badges emits and links to /p/{project}/_terminal/pane/{pane}. The twelve home_page_* router tests stay green and unedited, as do the existing hub unit tests except any whose call to bee_hub_card must pass the new argument.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work card-terminals` from 1 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "card-terminals" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-13T05:21:37.048Z), the work item declares no bee.areas.

area bee-cockpit:
  - [card-terminals-1] Cards on the cross-project Features board badge the terminal panes running in that feature's own checkout, joined on the checkout directory in server.rs and rendered by views.rs — feature-wide sync per the scribing stamp, 2 file(s) changed (trace .bee/cells/card-terminals-1.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 1 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.