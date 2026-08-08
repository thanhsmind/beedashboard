# Projects Home — Terminal Badges And Add Project — Context

**Feature slug:** projects-home
**Date:** 2026-08-07
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE | ORGANIZE

## Feature Boundary

The Projects page tells you, at a glance, which terminal panes are running inside
each registered project and lets you register a new project from the page itself.
It ends at the page and the one new register endpoint — the terminal views, the
pane strip, and the pane screen polling are untouched.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | Each project row carries one badge per terminal pane whose working directory falls inside that project's boundary. A badge shows a status glyph and the pane's agent name, or the shell name when the pane has no agent. | Ruled out a bare `3 terminals · 1 working` count: the user wants to know *which* agent, not how many. |
| D1a | **Clarifies D1.** The badge prints the pane's *program* — the herdr agent `kind`, or the literal `shell` for a pane with no agent — not the agent's `name` field. | `herdr::wire::Pane` carries no shell program field and `project_panes` leaves `name` empty for an agent-less pane, so D1's "shell name" had no data behind it. `kind` is what `pane_strip` already prints (`views.rs:394`) and what the user's own sketch drew. |
| D2 | Every pane in the boundary gets a badge, whatever its status — working, idle, done, blocked. Nothing is hidden. | Ruled out a working-only filter, which would also swallow blocked panes, the ones most worth seeing. |
| D3 | A badge is a link to that pane's terminal view (`/p/:id/_terminal/pane/:pane_id`) — the same destination the pane strip already links to. | — |
| D4 | Badges render server-side at page load. No polling endpoint, no client-side refresh; a refresh is a page reload. | Ruled out a ~3s poll — keeps the home page free of new endpoints and JS. |
| D5 | Worktree/branch rows carry badges under the same rule as parent rows: boundary match against that row's own root path. | Branch rows are registered projects with their own root; a pane inside a worktree belongs to the worktree row, not the parent. |
| D6 | When the terminal feature is off, or the herdr snapshot is unavailable, project rows render exactly as they do today — no badges, no error, no empty slot. | The Projects page must not depend on herdr being up. |
| D7 | The page gets an add-project form: one field, an absolute path. The project name is derived from the directory name. | Ruled out a path+name pair (the folder name is already the name in use) and a server-side directory browser (it would expose the filesystem over HTTP). |
| D8 | The form posts to a new register endpoint, which registers the path and returns to the refreshed project list. | Mirrors the existing `POST /api/projects/:id/unregister` form-post-and-redirect shape rather than introducing a fetch path. |
| ~~D9~~ | ~~The register endpoint accepts any path the server process can read: no allowed-roots list, no loopback-only guard. Same reach as `mdview register <dir>`.~~ **Superseded by D9a on 2026-08-07.** | ~~The user was shown that anyone reaching the unauthenticated port can then register and read markdown under any readable directory, and chose CLI parity anyway.~~ |
| D9a | **Supersedes D9.** The register endpoint still has no allowed-roots list and no loopback-only guard — any path is fair game — with two exceptions. It refuses a root that sits on the repository's existing `paths_boundary::hard_deny_list`, and it pre-flights the tree before indexing, refusing a root whose markdown count or walk time exceeds a fixed budget. | Review found `Engine::register` indexes inline through an uncapped, hidden-file-including `WalkBuilder` (`mdview-core/src/indexer.rs:88`), so one anonymous `POST path=/` walked the whole readable filesystem into sqlite; and that `register` consults no deny list, so `~/.ssh` or `/etc` could be registered and then served and grepped unauthenticated. The user chose the deny list plus the cap over a loopback-only guard, keeping CLI-like freedom of path while removing the runaway walk and the credential-directory case. |
| D9b | **Extends D9a.** The register route refuses a root that *contains* a hard-deny-listed directory as well as one that sits inside it. In practice that adds `/`, `/home` and `$HOME` to the refusals; `~/projects/whatever` still registers. | `Boundary::new` only answers "is this root inside a denied root", so `POST path=$HOME` passed the gate and then indexed markdown under `~/.ssh`, `~/.aws` and `~/.gnupg`, which `/p/:id/_search` greps unauthenticated — the credential-directory case D9a exists to remove. Ruled out recording it as a known hole, and ruled out switching the walker to `hidden(true)`, which would change CLI and existing-project indexing behaviour. |
| D10 | A path that does not exist, is not a directory, or is already registered is refused with a message shown on the Projects page. The list is never silently unchanged. | — |

### Agent's Discretion

Badge glyphs, colours, and layout within the row; the wording of the refusal
messages under D10; the request/response shape of the register endpoint;
whether the derived name is de-duplicated against an existing project name;
the exact numbers in D9a's pre-flight budget.
Constraint: reuse the existing status glyph vocabulary and row styling rather
than inventing a second visual language.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Badge | One small element on a project row standing for one terminal pane inside that project. |
| Boundary | The existing `paths_boundary::Boundary` containment test between a pane's working directory and a project root. |
| Pane | A herdr terminal pane, agent-backed or a plain shell. |

## Specific Ideas And References

- The user's sketch of a badge row under the project name:
  `beedashboard  118 markdown files · 20h ago` / `● claude  ○ shell  ✓ codex` —
  badges sit under the row's meta line, not inline with it.

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/mdview/src/server.rs:1544` — `project_panes()` already joins the herdr
  snapshot to one project's boundary and returns pane views; the badge data is
  this function's output.
- `crates/mdview/src/views.rs:431` — `TerminalPaneView` (`pane_id, kind, name,
  status, title, cwd, workspace, tab`) carries every field a badge needs.
- `crates/mdview/src/views.rs:372` — `pane_strip()` renders per-pane links with
  status glyphs; the badge is the same vocabulary at row scale.
- `crates/mdview-core/src/engine.rs:114` — `Engine::register(root, name)` is the
  add-project behaviour, already written and CLI-exercised.

### Established Patterns

- Server-rendered markup built with `format!` in `views.rs` — no template engine,
  no client render for the project list.
- Mutating project actions are HTML form posts, not fetch: see the unregister
  form at `views.rs:134`.

### Integration Points

- `crates/mdview/src/server.rs:268` — `index_page`, which lists projects and file
  counts; badges and the add form enter here.
- `crates/mdview/src/views.rs:91` — `project_list_page`, the markup for the list,
  including the worktree nesting loop at `views.rs:108`.
- `crates/mdview/src/server.rs:183` — the router table; the register route joins it.

## Canonical References

- `crates/mdview/src/herdr/wire.rs:132` — `Pane`, whose `cwd` / `foreground_cwd`
  are the working directory a badge is matched on.
- `crates/mdview/src/server.rs:321` — the note that `/api/projects` deliberately
  omits `root_path` because the route is unauthenticated; D9 knowingly departs
  from the caution behind it.

## Outstanding Questions

### Deferred To Planning

- [ ] Whether `index_page` can take one herdr snapshot and match it against every
      project in one pass, rather than calling `project_panes()` per project —
      a read of `project_panes()` and `Snapshot` answers it.
- [ ] How a register failure message is carried back to the Projects page in a
      redirect-based flow — query parameter, flash, or rendering the page directly
      from the POST handler.

## Deferred Ideas

- Live-updating badges — explicitly ruled out for now by D4, not by feasibility.
- Registering a project from the terminal side (a pane in an unregistered
  directory offering "register this") — a different entry point, out of scope.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
