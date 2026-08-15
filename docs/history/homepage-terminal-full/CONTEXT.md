# Homepage Terminal, Full Controls — Context

**Feature slug:** homepage-terminal-full
**Date:** 2026-08-15
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE | RUN

## Feature Boundary

The home page's Terminals tab renders the same terminal surface the standalone
terminal page renders — its pane switcher, its history controls, its terminal
creation controls, and its agent switch drawer — instead of the reduced surface
it has today. It ends there: the standalone terminal page, the transcript page,
and the unassigned terminal page keep their own behaviour unchanged, and no new
route is added.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | The Terminals tab renders the standalone page's own controls: the `pane_bar` pane switcher (which carries both the wide strip and the narrow-screen collapsed menu, and the terminal creation controls inside it), the history controls (Older / Newer / Live), and the agent switch drawer. | The standalone page is already complete and already collapses on a phone; keeping two different control sets for one thing is debt. |
| D2 | The one-line `<select>` shipped by `terminals-pane-select` is removed, along with its change listener and its CSS. `pane_bar` replaces it. | Its whole reason was compactness, which `pane_bar`'s own narrow-screen menu already provides — and `pane_bar` links are real links needing no scripting. |
| D3 | The tab does **not** render the project topbar or the Overview / Terminal / Transcript project nav. | The home page already has its own topbar and its own Kanban / Projects / Terminals strip; both together are two stacked navigations. |
| D4 | Everything the tab does today about *selection* holds unchanged: an absent pane takes the first pane in the existing order, a named pane that has gone renders its own "this terminal is gone" line with the full list still offered, and the two empty states — nothing running, and the agent host unreachable — read as they do now. | — |
| D5 | Every pane control keeps working from this tab, which means the `data-term-base` mechanism the tab already uses stays: the tab has no single project id, so each pane names its own base path. | The standalone page resolves its URLs from one `data-project-id` on the page root; the Terminals tab spans projects and cannot. |
| D6 | The file watcher never force-reloads a document that is showing a live terminal screen. Shipped ahead of this feature as `homepage-terminal-refresh`. | A reload dropped in-progress input and reset a running terminal. |
| D7 | The pane switcher's links stay inside the tab — `/?tab=terminals&pane=<id>` — and never jump to a project's own terminal page. `pane_bar`, `pane_strip` and `pane_tab` therefore take a per-pane link instead of composing one from a single `project_id`. | The tab spans projects and includes panes belonging to none, so one project id cannot build the links; and leaving the tab defeats the point of the tab serving itself. |
| D8 | The terminal creation controls belong to the selected pane's project. When the selected pane belongs to no project, the creation controls are not rendered. | Creating a terminal needs a project, and the selected pane is the only context the tab has. |
| D9 | `screenUrl` in `app.js` honours a per-pane base in the history-scroll branch too, not only in the poll branch. This also fixes an existing defect: the unassigned terminal page renders history controls but carries no `data-project-id`, so its buttons build `/p/null/_terminal/<pane>/screen`. | One change serves both; the tab needs it and the unassigned page is broken without it. |

### Agent's Discretion

- Whether the reused renderers are called as-is or gain a parameter, so long as
  the standalone page's output is unchanged.
- Where the agent switch drawer sits in the tab's markup.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Pane switcher | The control listing every agent pane and moving between them. On the standalone page this is `pane_bar`: a strip of links on a wide screen, a collapsed menu on a narrow one, with the terminal creation controls beside it. |
| Full controls | The set the standalone page renders around one pane: screen viewport, history controls, movement and action keys, reply form, and image attach where the pane allows it. |

## Existing Code Context

### Reusable Assets

- `crates/waggledance/src/views.rs` — `pane_bar` (pane switcher, ~line 889), `pane_cards` (screen + history controls + `pane_controls`, ~line 1150), `terminal_create_controls` (~line 1314), `AGENT_SWITCH_DRAWER` (~line 1351). All already parameterized and shared with `unassigned_terminal_page` and `transcript_page`.
- `crates/waggledance/src/views.rs` — `terminal_page` (~line 1370) is the assembly this tab should mirror, minus D3's topbar and project nav.

### Integration Points

- `crates/waggledance/src/views.rs` — `terminals_tab` (~line 1016) and `screen_frame` (~line 1098), the reduced surface being replaced.
- `crates/waggledance/src/server.rs` — `index_page`, which builds the pane inventory feeding the tab.
- `crates/waggledance/assets/app.js` — the `.terminals-pane-select` change listener, removed with D2.

## What The Scout Found

Both deferred questions are answered, and the answer reshapes the work: the
standalone page's controls are not drop-in reusable, because each of them
resolves URLs through one project.

- `pane_strip` / `pane_tab` / `pane_bar` take a `project_id` and bake
  `/p/{pid}/_{kind}/pane/{pane_id}` into every link — D7 replaces that with a
  per-pane link.
- `pane_cards` calls `pane_controls(..., None)`, so every control it renders
  falls back to a `data-project-id` on the page root, which this tab has not
  got. `pane_controls` itself already accepts a per-pane base; the caller has to
  start passing one.
- The history controls' JS calls `screenUrl(paneId, depth)` and never passes the
  base argument the function already accepts — D9.
- `terminal_create_controls` carries its own `data-project-id` and its script
  reads it at click time, which is why D8 ties it to the selected pane's project.
- `AGENT_SWITCH_DRAWER` is the one clean lift: static markup filled from
  `/api/agents`, a feed that already spans every project.

## Outstanding Questions

None blocking. Everything the scout raised is settled in D7-D9.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, and deferred-to-planning questions.
