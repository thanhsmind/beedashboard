---
artifact_contract: bee-plan/v1
mode: standard
---

# Plan: Homepage Terminal, Full Controls

Mode: `standard` — 2 risk flags: covered-contract-change, proof-weakening
Why this is the least workflow that protects the work: the change is small in
intent but touches four renderers shared by three other pages, so the protection
that matters is proving those pages' output is byte-identical before the tab is
rebuilt on top of them.

## Requirements (from CONTEXT.md)

- **D1** — the tab renders the standalone page's controls: pane switcher, history controls, creation controls, agent drawer.
- **D2** — the one-line select, its change listener and its CSS are removed.
- **D3** — no project topbar and no Overview/Terminal/Transcript nav.
- **D4** — selection behaviour and both empty states are unchanged.
- **D5** — each pane names its own base path; the tab has no single project id.
- **D6** — already shipped: the watcher never reloads a document showing a live screen.
- **D7** — the switcher's links stay in the tab; `pane_bar`/`pane_strip`/`pane_tab` take a per-pane link.
- **D8** — creation controls belong to the selected pane's project, and vanish when it has none.
- **D9** — `screenUrl` honours a per-pane base in the history-scroll branch, fixing the unassigned page too.

## Discovery

The standalone page's parts are not drop-in reusable: `pane_strip`/`pane_tab`/
`pane_bar` compose every href from one `project_id`, `pane_cards` calls
`pane_controls(..., None)` so its controls resolve through a page-root
`data-project-id`, the history buttons call `screenUrl(paneId, depth)` without
the base argument the function already accepts, and `terminal_create_controls`
carries its own `data-project-id` read by its script at click time. Only
`AGENT_SWITCH_DRAWER` lifts cleanly — static markup filled from `/api/agents`,
a feed that already spans every project.

Three pages consume these renderers today — the project terminal page, the
transcript page, and the unassigned terminal page — so every signature change
has to leave their output identical. One of them is already broken: the
unassigned page renders history controls with no `data-project-id` above them,
so its buttons build `/p/null/_terminal/<pane>/screen`. D9 fixes that on the way
through.

## Approach

Two cells. The first makes the shared renderers able to serve a page that has no
single project, changing no existing page's output except the one that is
already broken. The second rebuilds the tab on top of them.

Recommended path:

1. Give `pane_tab`, `pane_strip` and `pane_bar` a per-pane link instead of a
   `project_id` they compose one from (D7), and let `pane_cards` pass a per-pane
   base down to `pane_controls` (D5). The existing callers keep their current
   output by supplying exactly the links and bases they build today.
2. Pass the base through `screenUrl`'s history-scroll branch (D9). The
   unassigned page's buttons start working; nothing else changes.
3. Rebuild `terminals_tab` as the standalone page's assembly minus D3's topbar
   and nav: switcher, then the selected pane's card with its history controls
   and pane controls, then the creation controls when the selected pane has a
   project (D8), then the agent drawer. Delete the select, its listener and its
   CSS (D2).

Rejected alternatives:

- Give the tab a synthetic `data-project-id` — rejected: the tab genuinely spans projects and includes panes belonging to none, so any single value is a lie that breaks half the controls.
- Duplicate the renderers for the tab — rejected: two control sets for one thing is the debt this feature exists to remove.
- Have the switcher link to each project's own terminal page — rejected by D7; it leaves the tab.

Risk map:

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| `pane_bar` / `pane_strip` / `pane_tab` signature change | MEDIUM | Three pages render them; a wrong link is invisible until clicked | Existing page tests must pass untouched; assert the standalone page's links are unchanged |
| `pane_cards` base threading | MEDIUM | Its controls silently fall back to a page-root attribute when the base is absent | A test that the tab's controls carry `data-term-base` and the standalone page's do not |
| `screenUrl` history branch | LOW | Currently builds `/p/null/...` on one page | A test that the unassigned page's history buttons resolve without a project id |
| Removing the select | LOW | Shipped one commit ago with its own tests | Those tests are rewritten for the switcher, not deleted |

## Shape

| Phase | What Changes | Why Now | Demo | Unlocks |
|---|---|---|---|---|
| 1 — Renderers take a per-pane link and base | `pane_tab`/`pane_strip`/`pane_bar` take a link; `pane_cards` threads a base into `pane_controls`; `screenUrl` honours the base in its history branch | The tab cannot be assembled until its parts stop assuming one project; and this is where the existing pages must be proved unchanged | Every existing page renders exactly as before, and the unassigned page's Older/Newer/Live buttons work for the first time | Cell 2 |
| 2 — The tab is rebuilt on those parts | `terminals_tab` renders switcher, pane card with history controls, creation controls per D8, agent drawer; the select and its listener and CSS go | Only now can the tab be assembled from parts that serve it | The Terminals tab switches panes, scrolls history, sends keys and replies, creates a terminal in the selected pane's project, and opens the agent drawer | — |

## Test matrix

**Happy path** — the standalone terminal page, the transcript page and the
unassigned page render their existing markup unchanged; the Terminals tab
renders the switcher, one pane card with history controls, and the agent
drawer; the tab's controls each carry their pane's own base; choosing another
pane in the switcher stays on `/?tab=terminals`.

**Edge cases** — the selected pane belongs to no project, so the creation
controls are absent (D8) but every other control renders; the unassigned page's
history buttons build a URL with no project id in it (D9); an absent pane still
takes the first pane and a vanished pane still renders its gone line with the
full list (D4); no `.terminals-pane-select` markup, listener or CSS rule
survives anywhere (D2); the tab renders no project topbar and no
Overview/Terminal/Transcript nav (D3).

**Error paths** — the two empty states, nothing running and the agent host
unreachable, read as they do today (D4); a pane that vanishes between the
inventory and the render does not take the page down.

## Out of scope

- The standalone terminal page, the transcript page and the unassigned page's own layout — only the unassigned page's history-button URL changes, and only because it is broken.
- Any new HTTP route, and any change to `/api/agents`.
- Scrollback browsing beyond what the history controls already do.
