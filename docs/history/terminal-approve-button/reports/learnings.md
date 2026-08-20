# terminal-approve-button — Learnings (2026-08-20)

The build itself was one cell with no deviations; everything below came out
of the close, five days later.

- **A close-gate citation dies on a line wrap.** The routing door wants the
  feature slug and the decision id adjacent — `terminal-approve-button
  D1/D2/D3/D4`. Prose-wrapped so the slug ends one line and `D1` starts the
  next, the door still reported all four ids unrouted, with the same message
  as citing nothing at all. Two iterations went to the wrap. Keep slug and
  ids on one line, and treat "cited it and the door still refuses" as a
  formatting suspicion before a content one.
- **Both close sweeps attribute by open window, not by change.** The impact
  and doc sweeps name every decision logged while the feature sat active as
  "touched by" it. This feature stayed open from 2026-08-15 while
  `orchestrator-dispatch` and the pane-badge work logged their own
  decisions, so the close flagged 15 citing lines in documents this feature
  never touched. The correct answer was a recorded reason, not an edit —
  annotating those citations would have made accurate records wrong.
- **A recorded reason is itself a touch.** Logging that decision with
  `--relation touches:<six ids>` immediately created 15 fresh capture stubs,
  one per citing line, for exactly the finding it had just resolved. Cheap
  to flush, but a reason logged against many ids buys a matching pile of
  stubs — prefer `--relation none` when the decision is about the sweep
  rather than about the decisions it names.
- **A template's own boilerplate reads as deferral prose.** CONTEXT.md's
  `## Deferred Ideas` heading and the standard Handoff Note line both
  tripped the doc-deferral door, alongside the one real open idea. The real
  one earned a registered trigger; the two template lines needed a tagged
  decision, because there is nothing there to fix.
