# Bee Board — PM View — Context

**Feature slug:** bee-board-pm
**Date:** 2026-08-06
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE | READ

## Feature Boundary

The bee board at `/p/<project>/_bee` is rebuilt to answer a project manager's four
questions on one screen — what has been built, what is being worked on, what comes
next, where it is stuck — using only what already sits on disk under the project's
`.bee/` directory. It ends at the board page and the reader that feeds it: the two
existing detail pages keep their current shape, and nothing outside the bee surface
changes.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | The board adopts the source spec's information architecture and its attention rules, but keeps **English** labels throughout — the source spec's Vietnamese wording is not carried over. | The rest of mdview is English; one Vietnamese page would be an island. The spec's value is its structure, not its wording. |
| D2 | Delivery speed (velocity, cycle time), worktrees and workspaces **survive** the redesign, folded into the new layout — velocity as its own delivery-speed block, worktrees and workspaces inside the sessions panel. | The source spec was written against a repo whose board had none of these. Adopting a layout is not a reason to lose signal already earned. |
| D3 | Cards link to the existing `/p/:id/_bee/cell/:cell_id` and `/p/:id/_bee/feature/:feature` pages. The source spec's slide-over drawer is **not** built. | Real pages already carry shareable URLs and full test coverage; a drawer would duplicate them, add client JS, and cost the URL. The drawer exists in the source spec only because its prototype was a single self-contained file. |
| D4 | The board stays **file-read-only**: every number comes from reading `.bee/**` (and `docs/history/<feature>/promote-proposals.md`), never from executing `.bee/bin/bee`. Anything the source spec can only get from `bee status --json` is either derived from the files or dropped. | `docs/specs/bee-cockpit.md` ("Read-only, always") is a tested guarantee; mdview also serves projects it does not own a shell in. Shelling a CLI per page request would break both. |
| D5 | The board's top-level question order is fixed: **lifecycle stepper → headline numbers → what is being worked on now, beside what needs attention → all work by phase → supporting panels.** A section with nothing to show renders an honest empty state; it never disappears and never renders zeros as if they were measurements. | This ordering is the feature — it is what makes the page answer the four questions in reading order. |
| D6 | "Needs attention" is a **generated, severity-ordered list**, not a static panel: each rule fires independently on the data, the heaviest sits first, and every item names a suggested action. An empty list says so in one line. | Without this the page is a data dump; the attention list is what makes it a management view. |
| D7 | Independent review is presented as **user-invoked** wherever it appears — a lifecycle step the human triggers, never a stage the board implies runs on its own. | AGENTS.md: review is never an automatic stage. A board that renders it as pending work would teach the opposite. |
| D8 | Cells in state `dropped` count toward no progress denominator and no completion total. | Already the rule in `docs/specs/bee-cockpit.md`; restated because the new progress bar is a fresh place to get it wrong. |
| D9 | The board renders nothing that identifies a filesystem outside the project: no absolute paths, no transcript paths, no sibling worktree roots. | Existing tested invariant (`detail_pages_leak_no_absolute_path_or_fixture_root` and siblings); the redesign must not reopen it. |

### Agent's Discretion

Everything below the locked decisions is the agent's call, within these constraints:

- **Which fields to add to the reader.** The source spec's data model is a shopping
  list, not a contract. Add what D5/D6 actually need; leave the rest.
- **Visual design.** The source spec's colour tokens, spacing and component shapes are
  a reference to work from, not a pixel target — but semantic colour must never carry
  meaning alone (always a label or icon beside it), and the page must read correctly in
  both light and dark, with an explicit theme attribute beating the OS setting in both
  directions.
- **Panel composition.** How D2's velocity/worktrees/workspaces fold in is the agent's
  layout call, so long as none of their current information is lost.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Attention item | One generated finding on the board with a severity, a title, and a suggested action. Not a bee concept — it exists only on this page. |
| Lifecycle stepper | The row across the top showing where the active feature sits between exploring and independent review. |
| Phase column | One column of the by-phase board; its members are features, not cells. |
| Delivery speed | The existing velocity block: features shipped per period and median cycle time. |
| Knowledge debt | Capped work whose learnings were never recorded — scribing debt, the capture queue, and unapplied promote proposals, presented as one number a human can act on. |

## Specific Ideas And References

- `docs/history/bee-board-pm/pm-dashboard-spec.md` — the source spec the owner
  provided, verbatim. It targets a standalone Node tool reading the bee CLI; this
  feature applies its information architecture and its attention rules to mdview's
  existing page instead. Where the two disagree, this CONTEXT.md wins.
- The owner also pointed at a running prototype of that spec (a single self-contained
  HTML file). It is a look-and-feel reference only — its architecture (collector,
  injected JSON, drawer, SSE) is explicitly not adopted (D3, D4).

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/mdview-core/src/bee.rs` — the whole reader. `read_snapshot(root)` at
  `bee.rs:481` already folds `.bee/state.json`, `.bee/cells/*.json`,
  `.bee/backlog.jsonl`, `.bee/sessions/*.json`, `.bee/lanes/*.json`,
  `.bee/runtime/workspaces/*.json`, `.bee/runtime/worktree-grants.json` and
  `.bee/decisions.jsonl` into `BeeSnapshot`, and computes running workers in memory.
- `crates/mdview/src/views.rs:639` — `bee_board_page(project, snapshot) -> String`,
  the single render entry. Board CSS is inline in its own `<style>` block
  (`views.rs:654-684`); the shared `app.css` carries no `bee-` rules.
- `esc()` at `views.rs:2213` and `layout()` at `views.rs:12` — escaping and page chrome.

### Established Patterns

- Sections are built by small `bee_*_section` helpers returning HTML fragments
  (`views.rs:739` onward) and concatenated in `bee_board_page`. The redesign extends
  this pattern rather than replacing it.
- Board behaviour is tested through the HTTP route, in `mod bee_route_tests`
  (`crates/mdview/src/server.rs:2294`), with a standing pair of invariants per
  section: no leaked absolute paths, and the fixture's `.bee/` tree byte-identical
  after the request.

### Integration Points

- `crates/mdview/src/server.rs:234-236` — the three bee routes. Only the first one's
  rendering changes; the two detail routes keep their contract (D3).
- `crates/mdview/src/server.rs:751` — `is_bee_project`, the presence gate on the
  project home page. Unchanged.

## Canonical References

- `docs/specs/bee-cockpit.md` — the living behavioural spec for this area. Its
  read-only guarantee, its presence rule, and its dropped-cell rule are constraints on
  this work (D4, D8); its "four buckets" section is what the by-phase board replaces
  and must be updated at capture time.
- `docs/specs/reading-map.md:13` — the row naming this area's code entry points.
- `docs/history/bee-board-pm/pm-dashboard-spec.md` — the owner's source spec.

## Outstanding Questions

### Deferred To Planning

- [ ] Which additional `.bee/` files must the reader learn to read for D5/D6 —
      candidates seen on disk are `.bee/HANDOFF.json`, `.bee/capture-queue.jsonl`,
      `.bee/review-candidates.jsonl`, `.bee/reservations.json`, plus the
      `approved_gates`, `route`, `next_action` and `last_scribing_run` blocks already
      inside `.bee/state.json` but not yet parsed. Answered by reading the fixtures and
      a real store.
- [ ] Whether per-feature phase and gates can be read per lane file, or only for the
      one globally active feature — this decides whether the by-phase board can place
      every feature honestly or must mark some as unknown.
- [ ] How the existing fixture set must grow to cover the new sections, and whether
      any current test asserts on markup the redesign deletes.

## Deferred Ideas

- Live auto-refresh (SSE / file watching) so the board updates while a swarm runs —
  the source spec's phase 2. Deferred: it changes the page from a document into a
  connection, and the read-only guarantee needs a separate look under a watcher.
- A cross-project roll-up showing every registered bee project on one screen.
  Deferred: this feature is per-project by definition.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
