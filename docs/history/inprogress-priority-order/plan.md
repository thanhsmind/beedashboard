# In Progress first, and busiest first — Plan

**Feature:** inprogress-priority-order
**Lane:** standard (1 flag: covered-contract-change; story-sized behavior)
**Decisions:** `docs/history/inprogress-priority-order/CONTEXT.md` D1–D6

## Recorded deviation — worktree

AGENTS.md's worktree-first rule says code-touching feature work starts in
its own feature worktree. This feature runs in the EXISTING
`beedashboard--wt--card-collapse-inprogress` worktree instead. Reason: it
edits the same two files (`views.rs`, `server.rs`) that the finished but
still-unmerged card-collapse work rewrote, and a fresh worktree would
branch from a main that does not have that change — a guaranteed conflict
on the exact functions this feature edits, for no isolation gain. Both
features land together in one `bee worktree merge`.

## Shape

Two cells, strictly serial — both touch `crates/waggledance/src/views.rs`,
so there is no concurrency to win here.

### Cell 1 — pane data reaches the per-project board (D5)

The prerequisite: today `bee_render_hub_section` passes an empty pane slice
to every card (`views.rs:2696`), so D2's terminal half would be a no-op on
that board.

- `bee_board` (`server.rs:1433`) currently reads only
  `waggledance_core::bee::read_snapshot(&project.root_path)`. It gains the
  herdr snapshot (`st.herdr.snapshot().await`, the pattern at
  `server.rs:883`) and one `read_rollup(&[project.root_path.clone()])`
  (`bee.rs:1165`), then builds the feature→panes map through the existing
  `project_feature_panes(snapshot.as_ref(), &project, &rollup)`
  (`server.rs:2823`) — the same join the homepage already uses, never a new
  one.
- `BeeProjectRollup` carries its own `.snapshot`, so the rollup read
  replaces the standalone `read_snapshot` call rather than adding a second
  disk pass. The rollup read follows the repo's existing rule that
  `read_rollup` runs inside `spawn_blocking`
  (asserted by `cross_project_rollup_calls_read_rollup_inside_spawn_blocking`,
  `server.rs:16598`).
- `views::bee_board_page` → `bee_feature_hub_section` →
  `bee_render_hub_section` each take the map through, and the card call at
  `views.rs:2680` passes the feature's real panes instead of `&[]`.
- Herdr down / snapshot `None` flows through as an empty map, exactly as it
  already does on the homepage — the board still renders, just with no
  badges.

**Visible result:** In Progress cards on `/p/:id/_bee` show their terminal
badges, the same ones the homepage cards already show.

### Cell 2 — the ordering and the blocked line (D1, D4, D6, D7, D8)

- One shared comparator, written once and used by both boards, over three
  tiers: a card with at least one `"blocked"` pane, then one with at least
  one `"working"` pane, then the rest (D7) — the same ranking
  `terminals_status_rank` (`server.rs:3014`) already applies in the Agents
  drawer. Within a tier: newest `last_activity` first, parsed as RFC 3339
  the way `bee_fmt_trace_time` (`views.rs:3936`) already parses it; a
  `None` or unparseable activity sorts last; feature name A→Z breaks every
  remaining tie. The existing
  `cross_project_finished_orders_timed_newest_first_then_untimed_alphabetically`
  sort (`views.rs:3034-3039`) is the shape to imitate.
- Per-project board: sort the In Progress placements before rendering
  their cards.
- Homepage Kanban: the cross-project builder (`views.rs:2899-3005`) stops
  appending In Progress cards project-by-project and instead collects every
  project's In Progress cards into one list, sorts it with the same
  comparator, and renders it flat (D4). The four dense-row columns keep
  their current per-project grouping untouched (D6).
- Mobile order (D1): inside the existing `@media (max-width: 700px)` block
  (`views.rs:2170-2176`), give the In Progress group `order: -1`, selected
  through the `data-hub-group="in-progress"` attribute each group `<div>`
  already carries (`views.rs:3119`). No markup change, no new breakpoint,
  and wide screens render byte-identically.

- Blocked reason line (D8): `bee_hub_card` already renders an optional
  italic `bee-hub__reason` line (`views.rs`, the `reason_html` branch). A
  card whose panes include a `"blocked"` one gains a second such line
  reading exactly `Waiting on you — a terminal is blocked`, emitted after
  the existing gate/handoff reason rather than replacing it. The card
  already receives its `panes` slice, so this needs no new input — but note
  it must hold on BOTH boards, which is why cell 1 lands first.

**Visible result:** on a phone the In Progress column is the first thing on
the board; inside it the features with a blocked agent sit on top — each
saying so in its own line — then the ones with a running terminal, then the
most recently active.

## Smaller path check

Could the sort live in `bee_classify_features` (`views.rs:2541`), which
already sorts features by name, instead of in the two board builders?
No — it has no pane data and is shared by both boards including the
columns D6 excludes. Sorting at the point each board builds its In
Progress list is the smallest change that honors D4 and D6 together.

## Verify

`cargo test --workspace` green, plus new cases:

- Three cards, one with a `"blocked"` pane and the oldest activity, one
  with a `"working"` pane, one with neither and the newest activity: they
  render blocked, working, neither — proving the tiers beat activity (D7).
- A pane with status `"idle"`/`"done"`/`"unknown"`/`"shell"` earns no tier
  at all (D7).
- Equal tier, different activity: newer first; a `None`-activity card
  renders last; two identical cards fall back to name order (D7).
- A card with a `"blocked"` pane carries the line
  `Waiting on you — a terminal is blocked`; the same card with a gate
  reason carries both lines, gate first; a card with no blocked pane
  carries neither (D8).
- The homepage In Progress column interleaves two projects' cards by the
  comparator rather than grouping them (D4), while a dense-row column on
  the same render keeps its per-project grouping (D6).
- The `max-width: 700px` block carries the In Progress `order` rule, and no
  ordering rule leaks outside that media query (D1).
- The per-project board renders terminal badges for a feature whose
  checkout has panes, and renders none when the herdr snapshot is absent
  (D5).
