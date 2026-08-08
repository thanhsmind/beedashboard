---
artifact_contract: bee-plan/v1
mode: high-risk
approved_gate2: 2026-08-08
---

# Plan: Projects Home — Terminal Badges And Add Project

Mode: `high-risk` — 2 risk flags: audit-security, public-contracts. The
audit-security flag is a hard gate on its own: D9 adds a *mutating* route to a
server that carries no authentication, and D1 makes the unauthenticated home
page call herdr for the first time.

Why this is the least workflow that protects the work: the diff is small, but
both halves widen what an unauthenticated port can do and see — that widening
is what the plan and its proofs exist to pin down, so nobody later reads the
exposure as an accident.

## Requirements (from CONTEXT.md)

- **D1**, as clarified by **D1a** — each project row carries one badge per
  terminal pane inside that project's boundary: a status glyph plus the pane's
  *program* (`kind`, or the literal `shell`), never the agent's `name` field.
- **D2** — every pane in the boundary gets a badge, whatever its status.
- **D3** — a badge links to `/p/:id/_terminal/pane/:pane_id`.
- **D4** — badges render server-side at page load; no polling endpoint, no JS.
- **D5** — worktree/branch rows badge against their own root, not the parent's.
- **D6** — terminal off, or herdr unreachable: rows render exactly as today.
- **D7** — add-project form: one absolute-path field, name from the directory.
- **D8** — the form posts to a new register route and returns to the list.
- **D9a** (supersedes D9) — the register route still has no allowed-roots list
  and no loopback-only guard, with two exceptions: it refuses a root on
  `paths_boundary::hard_deny_list`, and it pre-flights the tree, refusing a
  root whose markdown count or walk time exceeds a fixed budget.
- **D10** — a path that is missing, not a directory, or already registered is
  refused with a message on the page; the list is never silently unchanged.

## Discovery

- `project_panes(&Snapshot, &Boundary) -> Vec<TerminalPaneView>`
  (`crates/mdview/src/server.rs:1544`) is already exactly the badge query, and
  `TerminalPaneView` (`views.rs:431`) already carries `pane_id`, `kind`,
  `status`. `status_pill()` (`views.rs:450`) already maps
  working/done/blocked/shell to the glyph vocabulary the sketch draws.
- `Engine::register(root, name)` is `ensure_project` (`engine.rs:114-116`) —
  **idempotent**, and it validates nothing. It returns the existing project on
  a root match (`engine.rs:49-55`) rather than failing; `Engine::canonical`
  (`engine.rs:44-46`) falls back to the raw path when `canonicalize` fails, and
  `index_file` swallows every metadata error into `Ok(None)`
  (`indexer.rs:43-52`), so `register("/does/not/exist")` and
  `register("/some/file.md")` both *succeed*. Every one of D10's refusals is
  the handler's own work; the engine raises none of them. The duplicate check
  must canonicalize first and go through `store.find_project_by_root`
  (`repository.rs:67`) — comparing the raw submitted string would miss a
  symlinked or trailing-slash variant, fall through to `ensure_project`, and
  return success with the list silently unchanged, which is exactly what D10
  forbids.
- **The P1 this plan exists to fix.** `ensure_project` calls
  `IndexService::index_project` inline (`engine.rs:72-77`), which walks with
  `WalkBuilder` at `.hidden(false)`, no `max_depth`, no cap
  (`indexer.rs:88-107`), synchronously inside an async axum handler. Under the
  original D9 a single anonymous `POST path=/` walked the entire readable
  filesystem into sqlite. And `hard_deny_list()`
  (`paths_boundary.rs:66-83` — `/etc`, `/root`, `/var/lib`, `/proc`,
  `$HOME/{.ssh,.aws,.config,.gnupg,.kube,.docker}`) is consulted **only** by
  `Boundary::new`; `register` never asks it, while the walker's
  `.hidden(false)` means dotfile trees index rather than skip. D9a closes both.
- The unauthenticated home page today makes **no** herdr call by construction,
  and `unauthenticated_home_page_shows_unassigned_presence_only_and_leaks_nothing`
  (`server.rs:9429`) pins that no *unassigned* pane's name or cwd appears on
  `/`. D1 does not cross that test: an unassigned pane sits in no project and
  therefore gets no badge. What D1 does change is that `/` starts reporting
  which programs run in which registered project — already visible at
  `/p/:id/_terminal` under the same single switch since terminal-open-access
  (`terminal_page_inner` gates on `terminal_family_enabled` and nothing else,
  `server.rs:735-737`), and `/api/projects` already enumerates project ids
  unauthenticated, so badges gated on `terminal_family_enabled` change
  aggregation and discoverability, not the *class* of fact disclosed. That
  gating is load-bearing, not cosmetic.
- `SocketHerdr::call` (`herdr/socket.rs:198-217`) has no timeout on connect,
  write, or read, and the router carries no `TimeoutLayer`. Today `/` is immune
  because it makes no herdr call; the moment it does, a herdr that accepts and
  never answers wedges the home page. D6's "unavailable" therefore has to cover
  the hang, not only the refused connection.
- Route tests live in `mod bee_route_tests` (`server.rs:2416`), drive the real
  `router()` with `tower::ServiceExt::oneshot`, and get panes from
  `FakeHerdr` (`herdr/fake.rs:26`) seeded via `agent_start`. Helpers:
  `build_state_with_dir`, `register`, `get`, `body_string`
  (`server.rs:2475-2526`). POST requests are built inline; there is no shared
  POST helper.
- `Engine::unregister` and `POST /api/projects/:id/unregister` have **zero**
  test coverage today. Not this feature's debt, but it means the existing
  project-mutation route offers no test to copy — the register route's tests
  are written from scratch.
- Evidence command: `cargo test --workspace`.

## Approach

Recommended path: extend the existing server-rendered list rather than
introducing any client-side data path (D4). `index_page` takes **one** herdr
snapshot, under a short `tokio::time::timeout`, and matches it against every
project in that one pass. The per-project idiom is already written — lift
`terminal_page_inner`'s three lines verbatim (`server.rs:747-749`:
`Boundary::new(vec![root]).map(|b| project_panes(&snap, &b)).unwrap_or_default()`),
which is what gives one project's unconstructable boundary an empty badge list
without touching any other row. (Not `unassigned_panes` — that one returns
`Vec::new()` for the *whole group* on any `Err` (`server.rs:1638-1644`), which
is the opposite semantics and the wrong precedent to copy here.) The badge
markup is a sibling of `proj-row__link`, never nested inside it — an anchor
inside an anchor is invalid HTML and browsers unnest it, which would break the
row link itself (D3).

The register route mirrors the unregister route's form-post shape
(`views.rs:140`) so both project mutations read the same way. Its handler owns
the whole D9a/D10 pre-flight, in order: canonicalize the submitted path;
refuse a non-absolute or missing or non-directory path; refuse a root on
`hard_deny_list`; refuse a root already in `store.find_project_by_root`;
pre-flight the tree with the same `WalkBuilder` settings the indexer uses,
aborting at the file cap or the wall-clock budget; only then call
`engine.register`, off the request thread via `spawn_blocking`.

Rejected alternatives:

- A JSON endpoint plus a poll for live badges — ruled out by D4.
- One combined `Boundary` over every registered root — fails closed as a whole
  if any single root is invalid; the per-project loop is why `unassigned_panes`
  is written the way it is.
- Echoing the rejected path back into the page as free text — a reflected
  value on an unauthenticated page; fixed error codes carry the same
  information with nothing to inject.
- Doing the "already registered" check, the deny-list check, or the cap inside
  `Engine::register` — that would change CLI behavior, which no decision here
  asked for. D9a's guards live in the route handler.
- Loopback-only on the register route — offered twice, declined twice.

Risk map:

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| Badges on `/` | MEDIUM | First herdr call from an unauthenticated page; a missed switch check turns the home page into a host-wide agent inventory | Route test: switch off ⇒ byte-identical rows, no badge markup; switch on ⇒ badges only for in-boundary panes |
| Boundary matching per row | MEDIUM | A leak here shows project A's panes on project B's row, or a worktree's panes on its parent | Route test with a parent, its worktree, and a sibling project, each with its own pane |
| Register: what registration *grants* | HIGH | Registering is not bookkeeping — it makes a tree readable: `project_path` serves any indexed file unauthenticated (`server.rs:2050-2070`) and `/p/:id/_search` greps its content (`server.rs:200`). This, not path shape, is D9a's real exposure | Route test: a deny-listed root is refused, so the one class of tree the codebase already calls off-limits cannot be turned into an unauthenticated read grant |
| Register: unbounded work | HIGH | `POST path=/` walked the whole filesystem into sqlite inline (`indexer.rs:88-107`, `engine.rs:72-77`) | Route test: a tree over the file cap is refused and registers nothing; the indexing call runs off the request thread |
| Register: path validation | MEDIUM | The engine validates nothing — missing paths and plain files both register successfully today | Route tests: relative, empty, `..`, missing, file-not-directory, duplicate — each refused with the list length unchanged |
| Error surfacing | LOW | A reflected message on an unauthenticated page | Fixed codes only; test asserts the rejected path string never appears in the body |
| herdr down or hung | MEDIUM | `SocketHerdr::call` has no timeout (`herdr/socket.rs:198-217`) and the router has no timeout layer, so a hung daemon would wedge `/` once it starts calling herdr | Route test with `FakeHerdr::set_available(false)` ⇒ 200 and plain rows; the snapshot call is wrapped in an explicit timeout |

Files, likely touch order: `crates/mdview/src/server.rs`,
`crates/mdview/src/views.rs`, `crates/mdview/assets/app.css`.

## Shape

Feature outcome: the Projects page answers "what is running where" and can
register a new project without dropping to the CLI.

Repo-reality basis: both halves attach to code that already exists —
`project_panes` for the query, `Engine::register` for the write, the
unregister form for the interaction shape. Nothing here is a new subsystem.

| Epic | Capability / risk area | Why it exists | Slices | Proof needed |
|---|---|---|---|---|
| E1 | Read: pane badges on the project list | D1–D6 — the "what is running where" answer, and the first herdr call from an unauthenticated page | S1 | Switch gating, boundary partition, herdr-down fallback |
| E2 | Write: register a project from the page | D7, D8, D9a, D10 — the new mutating route, its refusals, and the bounded walk | S2 | Deny-list refusal, cap refusal, path validation, duplicate refusal, list-unchanged on every refusal |

Slice queue:

- **S1 — badges (current slice).** End-to-end and user-visible: one snapshot,
  per-project boundary match, badge markup, CSS, tests. No stubs.
- **S2 — add project.** Depends on S1 only through file overlap
  (`server.rs`, `views.rs`), not behavior. Runs after S1 caps.

Serial, not concurrent: both slices edit the same two files, which is the
named reason (AGENTS.md concurrency law).

## Test matrix

High-risk, so the applicable edge dimensions are written out rather than the
triad. Each cell's writer judges existing coverage first (`.bee/expertise/tests.md`)
and authors only the gap — `paths_boundary` containment is already pinned at
`crates/mdview-core/src/paths_boundary.rs:241-348` and is not re-proved here.

**S1 — badges**

| Dimension | Probe |
|---|---|
| Authorization / exposure | `terminal.enabled` off ⇒ `/` body carries no pane id, no program name, no badge markup |
| Boundary | Parent project, its worktree, and a sibling each hold one pane ⇒ each row badges only its own (D5) |
| Empty | A registered project with no pane ⇒ row renders as today, no empty badge container |
| Failure of a dependency | `FakeHerdr::set_available(false)` (`herdr/fake.rs:280`) ⇒ `snapshot()` is `Err` ⇒ 200, plain rows, no error text (D6) |
| Fail-closed | One project's root unconstructable as a `Boundary` ⇒ that row badges empty; other rows keep theirs. `Boundary::new` never stats, so an ordinary root always constructs — the fixture has to be a deny-listed root (`paths_boundary.rs:258-263`) or a relative one that `Engine::canonical` leaves relative |
| Contract | Each badge's `href` is `/p/{id}/_terminal/pane/{pane_id}` and resolves on the router (D3) |
| Markup validity | No `<a>` nested inside `proj-row__link` |
| State variety | A pane per status — working, idle, done, blocked, and an agent-less shell — all badge (D2). Note `status_pill` (`views.rs:450-462`) tints only done/working/blocked; `idle` and `shell` share the neutral dot and are told apart by the pill's text, so assert on text, not on the modifier class |
| Program, not name | An agent-less pane badges as `shell`; an agent pane badges its `kind`. The agent's `name` field appears nowhere in the body (D1a) |

**S2 — add project**

| Dimension | Probe |
|---|---|
| Happy path | POST an existing absolute directory ⇒ 303 to `/`, the row appears, name is the directory name (D7) |
| Input validation | Relative path, empty path, and a path containing `..` ⇒ refused, no project added |
| Type confusion | A path to a regular file ⇒ refused as not-a-directory |
| Idempotence | Registering an already-registered root ⇒ refused with the duplicate message, project count unchanged (D10 — `ensure_project` would otherwise succeed silently) |
| Aliasing | The duplicate refusal also fires for a symlink to, and a trailing-slash form of, an already-registered root — a raw-string comparison would let both through as silent successes |
| Deny list | `POST` with a root on `hard_deny_list` (e.g. `$HOME/.ssh`) ⇒ refused, nothing registered, nothing indexed (D9a) |
| Bounded work | A fixture tree over the markdown-file cap ⇒ refused, nothing registered; asserted by project count, not by timing |
| Error surface | The rejected path string never appears in the response body |
| Method | GET on the register route ⇒ 405 — confirmed reachable in this router by the existing `a_get_carrying_switch_values_in_its_query_changes_no_switch` (`server.rs:6870-6889`) |
| Recorded exposure | A test named for D9a asserting the route *does* register an ordinary path outside every existing project root, with no allow-list — the openness is pinned as deliberate, so a later reader cannot mistake it for an oversight |
| Unchanged on refusal | `engine.list_projects()` length is asserted before and after every refusal case |

## Out of scope

- Live-updating badges (D4 rules them out; deferred, not infeasible).
- Any authentication, allow-list, or loopback guard on the register route —
  D9a decided against all three. Its deny-list and cap are not an allow-list:
  every other path still registers. Adding a guard later is a new decision,
  not a fix.
- A general request timeout layer for the whole router. This plan wraps the
  one new herdr call `/` makes; the untimed `SocketHerdr::call` on every other
  terminal route is pre-existing and stays as it is.
- Test coverage for the existing unregister route — real debt, filed
  separately, not smuggled into this feature.
- Registering a project from the terminal side.
