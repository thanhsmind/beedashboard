---
feature: console-theme-kanban
started: 2026-08-22
status: locked
decisions: [b27a73c6, and the two logged beside it — see "Locked decisions"]
---

# Console theme + homepage kanban rebuild

## What the user asked for

Take the visual style set from `/home/thanhsmind/projects/AI/agent-orchestrator`
and make waggledance look like it, then rebuild the homepage kanban so its
elements match two supplied screenshots: a desktop board (sidebar, topbar,
four dotted columns of cards, a collapsed `ARCHIVE 72` bar) and a phone
screen (stat tiles, grouped sections, bottom tab bar).

The source style set is distilled in `style-digest.md` beside this file —
that digest is the reference for every value; the agent-orchestrator repo is
read-only and is never edited.

## Locked decisions

**D1 — The console look replaces Atelier everywhere.** (decision `b27a73c6`)
Waggledance ships one theme, and it is the agent-orchestrator look: a dark,
cool, compact developer console. Every surface renders in it — the homepage
board, project pages, and the doc reading pages. Atelier stops being the
shipped theme.

Consequence for the implementation: Atelier is a four-tier system whose theme
file is the single place that maps private primitives onto the semantic and
character token contract. Replacing the theme file therefore replaces the look
everywhere without touching a single `.fg-*` component rule. The component
layer (`contract.css`, `components.css`, `editorial.css`) stays; only the theme
adapter is swapped, and the page-local palette override that the board
currently carries is deleted so the board inherits the one theme.

**D2 — Cards render only elements backed by real data.** (decision logged
beside D1)
A PR chip, a test count, a comment count, an avatar, a checks line or a merge
action appears on a card only where bee's store actually holds that value, and
is omitted otherwise. No placeholder, zero, or em-dash stands in for a missing
source. The screenshots' PR rows and comment counts have no source in bee and
are therefore not rendered; cell counts, proof verdicts, worker names, worktree
branch and last-activity time do have sources and are.

**D3 — The phone screen is responsive CSS over the same markup.** (decision
logged beside D1)
The second screenshot is delivered as breakpoints on the existing homepage —
the stat tiles, the grouped sections and the bottom-anchored chrome collapse
out of the desktop markup. No second route, no separate mobile surface.

## Boundaries this feature inherits

- The cockpit is a **read-only** surface (`docs/specs/bee-cockpit.md`). The
  screenshots' `Merge PR` button is an action; under D2 and the read-only
  contract it renders as a *state* ("Ready to merge"), never as a control that
  writes. The mobile `+` FAB is dropped for the same reason.
- `docs/specs/bee-cockpit.md` already documents a stale three-group board
  ("Waiting on you / In Progress / Finished") while the code ships five groups.
  The spec is re-synced to whatever this feature lands, as part of this feature.
- Several tests assert **exact literal substrings** of the inline board CSS.
  They move in lockstep with the CSS; a restyle that quietly breaks them is a
  red base, not a passing one.

## Open — carried into planning, not blocking

- Column-to-status-colour mapping across five waggledance columns versus the
  screenshot's four.
- Whether the Finished column becomes the screenshot's collapsed `ARCHIVE n`
  bar spanning the board, which is the shape the screenshot actually shows.
