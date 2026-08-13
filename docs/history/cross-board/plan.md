---
artifact_contract: bee-plan/v1
mode: standard
# approved_gate2: <unset until approval>
---

# Plan: Cross-Project Board

Mode: `standard` — 1 risk flag: covered-contract-change (twelve existing
`home_page_*` tests assert the markup `/` emits today).
Why this is the least workflow that protects the work: the change is a single
demo-able slice, but it re-homes the site's front door — twelve router tests
pin its markup and sixteen unit tests pin the column rules it borrows — and it
puts eight synchronous filesystem walks on that page's hot path. That
combination earns a written shape, not a merged tiny gate.

**Named deviation (form rule).** The Shape section below is a cell table, not
the phase plan the template prescribes for standard. The work has one
observable milestone — `/` shows the roll-up — so splitting it into phases
would invent milestones that do not exist. The slice is stated once and its
three cells are named with their seams and dependency.

## Requirements (from CONTEXT.md)

- **D1** — the roll-up lives on `/`, ordered Live, then Features, then the
  existing project list.
- **D2** — no new route; `/p/:id/_bee` is untouched.
- **D3** — the same three columns, same order, same names: Waiting on you,
  In Progress, Finished.
- **D4** — flat lists inside each column; never per-project blocks, never one
  row per project.
- **D5** — every card and every Finished row carries its project's name.
- **D7** — Finished shows 10 rows, the rest behind "Show 10 more · N left".
- **D8** — a project qualifies only when it is registered AND its root has
  `.bee/`; non-qualifying projects still appear in the list below.
- **D9** — when nothing qualifies, Live and Features are absent and `/` reads
  exactly as it does today.
- **D10** (supersedes D6) — Finished orders in two blocks: features with a ship
  time first, most recent first, each showing that time; then features without
  one, alphabetically. D7's cap applies to the combined sequence.

## Discovery

Measured against the live registry rather than assumed.

- **The registry is small and the per-project cost is not.** 10 registered
  projects, 8 qualifying under D8; between them 194 live cells and ~306
  `docs/history/<feature>/` directories, one project holding 204 of those on
  its own. Evidence: `sqlite3 ~/.mdview/registry.db "select root_path from
  projects;"` plus a `.bee/` presence and directory count per root.
- **`read_snapshot` is blocking and uncached.** `bee.rs:932` does ~10 fixed
  reads, one read per cell, and one directory read per distinct feature name.
  Both existing callers (`server.rs:1268`, `server.rs:3081`) call it directly
  on the async task; `spawn_blocking` is used elsewhere in the same file
  (`server.rs:294`, `1027`, `1085`) but not around either snapshot read.
- **`mdview-core` may not learn about threads-with-a-runtime.**
  `crates/mdview-core/src/bee.rs:3604-3612` is a test that fails if `axum`,
  `tokio`, or `hyper` appears in that crate's manifest; the module doc states
  the rule at `bee.rs:4`. Off-thread and concurrent execution therefore belongs
  in `crates/mdview`.
- **The Finished order is alphabetical today,** not chronological:
  `views.rs:1901-1902` sorts `phase_board` by feature slug and both the card
  groups and the placed Finished rows inherit that order; the archive-only tail
  is sorted alphabetically again at `views.rs:1999`. This is what forced D10.
- **D10's ship time comes from the archive, not from `snapshot.shipped`.** The
  Finished column is populated mostly from `list_archived_feature_dirs`
  (`views.rs:1898-1899`), but `snapshot.shipped` is computed from
  `.bee/cells/*.json` only (`bee.rs:950-960`, `bee.rs:994`) — the archive is
  excluded by construction, so joining on it would strand most finished
  features in D10's untimed tail. The real source is `trace.capped_at` on the
  archived cells themselves, latest-wins per feature — the same shape
  `feature_cell_span` already uses at `server.rs:3106`. Measured across all
  eight qualifying projects: 144 archived features, 346 archived cells, 140 of
  144 features carrying a usable time on every cell, and a full scan of all of
  it costs ~46 ms single-threaded in Python on a warm cache. The cost is real
  but small, and it buys D10 substantially its whole column.
- **The view layer is already unit-testable.** `bee_feature_hub_section`
  (`views.rs:1883`), `bee_hub_finished_row` (`views.rs:2209`), and
  `bee_hub_finished_rows` (`views.rs:2228`) are plain `fn(..) -> String` with
  sixteen `#[test]` cases calling them directly. `project_list_page` has no
  direct unit test — it is covered only through the router by twelve
  `home_page_*` tests in `server.rs` (13688–14339).

## Approach

**Recommended path.** Split the work at the seam that already exists: reading
is core's job, classifying and rendering is the view's job, composing and
scheduling is the server's. First give `mdview-core` a purely synchronous
roll-up read that returns, for a list of project roots handed to it, one
snapshot per root plus the archived-feature ship times D10 needs — no async
vocabulary anywhere in it, because a test forbids it. Then restructure the
column classification that `bee_feature_hub_section` performs today so it
returns classified *data* for one project instead of finished HTML, and add a
cross-project renderer that merges those per-project results into three flat
columns (D3, D4), tags each entry with its project (D5), sorts and caps the
merged sequence (D10, D7), and sums the three counters. Finally compose `/`:
the server applies D8, runs the roll-up off the async task and concurrently,
and emits Live, Features, then today's project list (D1), with both new
sections absent when nothing qualifies (D9).

The per-project board is not refactored into the cross-project renderer. It
keeps its own entry point (`bee_board_page`, `views.rs:1371`) and its rendered
output unchanged (D2); only the classification step underneath it is shared.

**Rejected alternatives.**

- Render eight per-project hub sections and stack them — violates D4, and
  concatenating eight alphabetical lists is not globally alphabetical, so D10's
  second block would be wrong.
- Cache snapshots behind a TTL — real, but it is a second feature with its own
  invalidation questions; the measured cost does not require it.
- Derive D10's ship time from the archive directory's mtime instead of reading
  the cells — nearly free, but a clone or a checkout rewrites every mtime, so
  the ordering would be silently wrong on a fresh machine.
- Read snapshots lazily from the browser after first paint — adds a route,
  which D2 forbids, and makes the page's first paint empty.

**Risk map.**

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| Classification restructure | MEDIUM | Sixteen unit tests pin the current column rules; the function's body is being reorganised around a new return type, not merely called differently | Those sixteen stay green; the twelve `feature_hub_*` router tests stay green; the per-project page's rendered output is unchanged |
| `/` markup contract | MEDIUM | Twelve `home_page_*` tests assert the page's rows, badges, and script selectors, including `home_page_script_selectors_match_the_markup_the_page_emits` (`server.rs:14339`) | `cargo test --workspace` green with those twelve unedited — D1 adds sections above the existing list rather than reordering inside it |
| Blocking reads on the async task | MEDIUM | Eight `read_snapshot` walks plus the archive scan, one project holding 204 feature directories | The read call sites on `/` are inside `spawn_blocking`, and a fixture test renders `/` with several qualifying projects |
| `list_archived_feature_dirs` in the view layer | MEDIUM | `views.rs:1898-1899` reads the filesystem from inside the renderer; called once per project that is eight more blocking reads on the async task | The cross-project renderer takes archived-feature data from the roll-up rather than reading it itself |
| Two renderer signatures change | LOW | `bee_hub_card` and `bee_hub_finished_row` gain a project label and, for the row, a ship time | Their four direct unit tests are updated to pass the new arguments and continue to assert the same behaviour, plus the new label and time |
| Duplicate feature slugs across projects | LOW | Two projects can both own a feature named `auth`; a flat list must keep them distinct rows with distinct links | An explicit test case |
| An unreadable or partial `.bee/` in one project | LOW | `read_snapshot` already returns `read_errors` rather than failing | A test that one broken project does not remove the others from `/` |

## Shape

One slice — `/` shows the cross-project roll-up end to end, with real data,
no stubs. Three cells; the third depends on the second because both write
`views.rs`.

| Cell | Seam | What changes | Depends on |
|---|---|---|---|
| `cross-board-1` | `crates/mdview-core/src/bee.rs` | A synchronous roll-up read: given project roots, return one `BeeSnapshot` per root together with each archived feature's ship time, taken as the latest `trace.capped_at` across that feature's archived cells and absent when any of them lacks one. No async, no threads-with-a-runtime — the framework-free guard at `bee.rs:3604` forbids it. D8's filter is not duplicated here; the caller passes the roots it already qualified | — |
| `cross-board-2` | `crates/mdview/src/views.rs` | `bee_feature_hub_section`'s classification is restructured to return classified per-project data instead of finished HTML, leaving the per-project page's output identical; a new cross-project Features section merges those results into three flat columns (D3, D4), labels every card and row with its project (D5), sorts and caps the merged sequence per D10 and D7, and sums the three counters. Archived-feature data comes from cell 1's roll-up, not from a filesystem read inside the renderer | `cross-board-1` |
| `cross-board-3` | `crates/mdview/src/views.rs`, `crates/mdview/src/server.rs` | A cross-project Live strip; `index_page` applies D8 through the existing `is_bee_project` rule (`server.rs:1257`), runs cell 1's roll-up inside `spawn_blocking` and concurrently across projects, and composes `/` as Live, Features, then today's project list rendered exactly as it is now (D1) — both new sections absent when no project qualifies (D9) | `cross-board-2` |

## Test matrix

The triad, at its smallest demonstrating size. Each cell's writer judges the
existing coverage first — sixteen `views.rs` unit tests and twelve `server.rs`
`home_page_*` tests already pin much of this — and authors only the gap.

**Happy path**

- Three qualifying projects with features in all three states: each feature
  lands in the column its per-project board would put it in, and carries its
  own project's name.
- Finished, mixed: features with a ship time come first, most recent first,
  showing that time; the rest follow alphabetically across all projects (D10).
- Finished with more than ten combined entries pages behind
  "Show 10 more · N left", the cap applied to the merged sequence (D7).
- The per-project board at `/p/:id/_bee` renders exactly as before (D2).
- `/` still lists every registered project below the new sections (D1).

**Edge cases**

- No qualifying project: neither section is emitted and `/` matches its
  current markup (D9).
- A qualifying project with `.bee/` but no features at all contributes nothing
  and breaks nothing.
- Every finished feature lacks a ship time: the whole column is the
  alphabetical block, still paged.
- A feature whose archived cells carry a mix of present and absent
  `capped_at` is treated as untimed, not as partially timed.
- The same feature slug in two projects renders as two rows with different
  project labels and different links.
- A registered path that no longer exists on disk (the registry holds two such
  stale worktree entries today) is treated as non-qualifying, not as an error.

**Error paths**

- One project's `.bee/` is unreadable or holds a corrupt cell: that project's
  readable content still appears, the other projects are unaffected, and `/`
  returns 200 rather than failing.
- The roll-up's filesystem work runs off the async task — asserted structurally
  at the call site rather than by a timeout, because a timeout around
  `spawn_blocking` abandons the thread instead of stopping the read.

## Out of scope

- Filtering or searching the cross-project board (deferred in CONTEXT.md).
- A cross-project Backlog and review panel (deferred in CONTEXT.md).
- Caching snapshots between requests.
- Any change to `/p/:id/_bee`, `/p/:id/_bee/cell/:id`, or
  `/p/:id/_bee/feature/:feature`.
- Reordering anything inside today's project list, including the suggestions
  block and the Unassigned presence marker.
- Cleaning stale worktree registrations out of the registry.
