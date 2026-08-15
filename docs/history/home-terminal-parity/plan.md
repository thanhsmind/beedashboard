# Home Terminal Parity — Plan

**Feature:** home-terminal-parity · **Lane:** standard · flags=1
(covered-contract-change) · files=3

Decision of record: the homepage Terminals tab gets the project terminal
page's own controls — the Agents drawer replaces the dropdown, plus history
scrolling, a pane status line, and create buttons. Drawer groups by project;
creating a pane targets the selected terminal's project. Logged 2026-08-15.

## Why it is not already there

`screen_frame` (views.rs:1102) deliberately renders the viewport alone, and
`terminals_tab` (views.rs:1020) switches panes through a plain `<select>`. The
project page composes more: `pane_cards` (views.rs:1154) adds the
`.term-scroll` history stack, `terminal_page` splices `AGENT_SWITCH_DRAWER`
(views.rs:1355) and `terminal_create_controls` (views.rs:1318).

Three facts decide the shape:

1. The drawer chrome is static and project-free — its JS keys off
   `#agent-drawer-toggle` and `[data-agent-drawer-list]` and polls
   `/api/agents`, which already returns `pane_id`, `project_id` and
   `project_name` per agent. It can therefore group by project and link to
   `/?tab=terminals&pane=<pane_id>` on the homepage while the project page
   keeps its status grouping and its own links.
2. The create script builds its URL from `data-project-id` on the
   `.term-create` box (views.rs:1283-1298) and posts to
   `/p/:id/_terminal/create/...`. A pane outside every project has no such
   route, so create renders only when the selected pane belongs to a project.
3. The scroll buttons call `screenUrl(paneId, depth)` without the third
   argument (app.js:1140, 1165, 1195), so they resolve through
   `main.fg-page[data-project-id]` — absent on the homepage. They must read
   the screen's own `data-term-base`, exactly as `pollOne` already does
   (app.js:1030).

## Slices

### Slice 1 — the screen area reaches parity (cell 1)

- `screen_frame` renders the `.term-scroll` older/newer/live stack that
  `pane_cards` renders, same markup and aria labels.
- A pane status line above the screen: project label, status pill, program,
  title — the identity that today lives only inside the dropdown's text.
- `assets/app.js`: the three scroll handlers pass the screen's
  `data-term-base` through `screenUrl`, leaving the project and unassigned
  pages on their existing path.

### Slice 2 — the switcher and create reach parity (cell 2)

- The `<select>` goes; the homepage renders the drawer chrome instead, marked
  as the homepage variant so the shared JS groups rows by project and links
  each row to `/?tab=terminals&pane=<pane_id>`.
- `terminal_create_controls` renders for the selected pane's project;
  omitted when the selected pane is unassigned.
- The homepage tab starts receiving the configured preset labels, the way
  `terminal_page` already does.

## Test scoping

`commands.test` = `cargo test --workspace`.

Seven homepage tab tests assert the `<select>` markup and the no-scroll screen
shape (server.rs:15601, 15651, 15709, 15761, 15813, 15874, 15955) — each
updated in place. New: the scroll stack present on the homepage with the
right base, the status line's contents, the drawer rendered in homepage mode,
create present for a project pane and absent for an unassigned one, and the
project page's own drawer and scroll behaviour unchanged.

## Cost if the shape is wrong

The create box carrying the wrong project id would start a pane in the wrong
place — the reason create is bound to the selected pane's project and hidden
when there is no project, rather than defaulting to something.

## Rollback

No new route, no new state. Reverting means restoring `screen_frame`'s
viewport-only body and the `<select>`; the drawer JS branch is additive.
