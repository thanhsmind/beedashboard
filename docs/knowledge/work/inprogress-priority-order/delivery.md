---
type: bee.delivery
title: inprogress-priority-order — delivery
description: "Delivery record for work item inprogress-priority-order: In Progress leads the board on a phone, and its cards order themselves by what is waiting."
timestamp: 2026-08-15
bee:
  id: inprogress-priority-order-delivery
  lifecycle: active
  areas: [web-interface, agent-terminal]
  required_context: [docs/specs/web-interface.md, docs/specs/agent-terminal.md]
  sources: [docs/history/inprogress-priority-order/CONTEXT.md, docs/history/inprogress-priority-order/plan.md, .bee/cells/archive/inprogress-priority-order/inprogress-priority-order-1.json, .bee/cells/archive/inprogress-priority-order/inprogress-priority-order-2.json]
---

# inprogress-priority-order — Delivery

## What shipped

Stacked on a phone, the five columns fell in their desktop order, which put
Todo above the one column that still carries cards. And within In Progress
the cards sat in whatever order the features happened to be named, so the
feature whose agent was standing still waiting for an answer could be
anywhere in the list.

Both are now ordered by what is actually happening.

- **In Progress leads on a narrow screen.** Below the board's existing
  narrow-screen width the In Progress column renders first, above Todo; the
  other four keep their relative order. Wide screens are untouched — the
  rule lives only inside the narrow-screen block.
- **A blocked agent rises to the top.** Cards sort in three tiers: a feature
  with at least one blocked terminal, then one with at least one working
  terminal, then everything else. Idle, done, unknown and plain shell panes
  earn no tier. This is the same blocked-before-working-before-rest ranking
  the Agents drawer already applies, reused rather than reinvented.
- **Then recency.** Inside a tier the most recently active feature comes
  first, a feature with no recorded activity goes last, and two otherwise
  equal cards fall back to feature name so the order never shuffles between
  renders.
- **A blocked card says so.** A card with a blocked terminal carries its own
  line reading `Waiting on you — a terminal is blocked`. A card that was
  already waiting on a gate carries both lines, the gate line first: two
  different things can be waiting on the user at once and neither should
  swallow the other.
- **One list, not one list per project.** The home page's In Progress column
  used to walk project by project. It is now a single list sorted across
  every project; a card still names its own project through its accent
  border and its subtitle. The four one-line columns keep their per-project
  grouping.
- **The project's own board caught up.** Cards on a project's own feature
  board had never been given terminal data, so they showed no badges and the
  terminal tiers would have been dead there. That board now resolves panes
  through the same feature-to-terminal join the home page already used —
  never a second join — and the rollup read it needs replaced the separate
  store read it was already doing, so the page costs no extra disk pass.

With the terminal service unavailable the whole thing degrades to no badges
and no tiers, and the boards still render.

## Verify

`cargo test --workspace` green at 976, up from 964. Cases cover three cards
where the blocked one has the oldest activity and the untiered one the
newest, proving tiers beat recency; idle, done, unknown and shell earning no
tier; recency, missing activity and name fallback inside one tier; the home
page column interleaving two projects while a one-line column on the same
render keeps its grouping; the blocked line's exact wording, alone and
alongside a gate line; the narrow-screen rule being present and no ordering
rule leaking outside that block; and the project board drawing badges with
terminals present and none when the terminal service is absent.

Confirmed against the running daemon: the served stylesheet carries the
narrow-screen ordering rule.

## Deviations

Built in the previous feature's worktree rather than its own. The two
features rewrite the same functions, and a fresh worktree would have branched
from a main without the first one — a guaranteed conflict for no isolation.
Both landed in one merge.

## Provenance

Written from the capped traces of `inprogress-priority-order-1` and `-2` and
the locked decisions in `docs/history/inprogress-priority-order/CONTEXT.md`,
where the first ordering rule counted only working terminals and was
superseded once blocked agents were brought in. Orders the cards
[card-collapse-inprogress](../card-collapse-inprogress/delivery.md) collapsed,
in the column [kanban-columns](../kanban-columns/index.md) left as the only
card-bearing one, using the pane data
[card-terminals](../card-terminals/index.md) first put on a card.
