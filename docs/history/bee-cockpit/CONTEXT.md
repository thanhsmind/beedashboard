# Bee Cockpit — Context

**Feature slug:** bee-cockpit
**Date:** 2026-08-05
**Shaping session:** complete
**Scope:** Standard
**Domain types:** SEE | READ

## Feature Boundary

A read-only dashboard surface inside mdview that, for one registered project
containing a `.bee/` directory, shows what bee is doing there — backlog, cells by
status, lanes, sessions, and ship velocity — with click-through to the detail of
each. It ends at the boundary of a single project: the roll-up across all
registered projects is phase 2, and nothing in this feature writes bee state.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | The cockpit is a new surface inside the existing mdview Rust workspace — not greenfield, not a separate repo. mdview source stays. | mdview already owns the project concept, a registry, an axum server, the atelier design system and single-binary distribution. A bee dashboard is one more view over a project mdview already knows. |
| D2 | Project discovery reuses mdview's existing registry at `~/.mdview/registry.db`. No new `~/.config/bee` registry is built. | The user opts a project in by registering it with mdview. That already excludes the worktree dirs (`*--wt--*`), `target/`, and scratch dirs that a raw filesystem scan wrongly picks up — 5 of 39 `.bee/` hits on this machine are such noise. |
| D3 | A registered project shows the bee surface only when its `root_path` contains a `.bee/` directory. A registered project without `.bee/` behaves exactly as it does today — no bee tab, no empty panel. | The bee surface is additive; non-bee projects must not gain a broken affordance. |
| D4 | The cockpit is strictly read-only. It reads `.bee/` state and never writes it: no gate approval, no cell claim, no backlog edit, no session kill. | Gates belong to the user through the bee CLI, and live sessions own their own state. A writing dashboard would race a running session. |
| D5 | A feature counts as **shipped** when every cell belonging to it is `capped` AND its worktree is merged into the main branch. Cycle time runs from the first cell's `trace.claimed_at` to the merge commit date. | The user chose the milestone closest to real delivery. Capped-only would count unmerged work as shipped; `docs/history/<feature>/` directory dates only record when scribing committed, not when work started. |
| D6 | Phase 1 delivers the per-project view only. The cross-project roll-up over all registered projects is a separate, later feature. | The user's words: "trước tiên phát triển dashboard cho từng project được đăng ký này". |
| D7 | Cells display in **four** buckets, not three: **Doing** (`claimed`), **Waiting** (`open`), **Stuck** (`blocked`, rendered red as its own bucket), **Done** (`capped`). `dropped` cells are hidden from the default view. | Stuck work is what the user most needs to see at a glance, so it must never hide inside Waiting. A dropped cell never shipped, so counting it as Done would inflate the numbers. |
| D8 | A project counts as **active** when it has at least one cell in `open` or `claimed` status — unfinished work, not session liveness. | Session heartbeat measures recent attention, not outstanding work, and `state.json.phase` goes stale when a session dies mid-run. |
| D9 | Statistics read only live cells under `.bee/cells/*.json`. The `.bee/cells/archive/` tree is not read in phase 1. | Measured across all five registered projects at shaping time: 200 live cells versus 5 archived, because `bee close` is barely used. Reading archives would add cost for 2.4% more data. Revisit if `bee close` becomes routine. |
| D10 | **Supersedes the merge clause of D5.** A feature is **shipped** when every one of its **non-dropped** cells is `capped`. A worktree merge into main is **not** required, and a dropped cell never blocks shipped status. | Measured on beehive: 8 of 25 features are release or docs-lane work that AGENTS.md permits to land in the main checkout with no worktree at all. Requiring a merge commit marked them never-shipped and undercounted velocity by about a third. It also resolves the D5×D7 contradiction — `dispatch-worktree` is capped-plus-dropped and now counts as shipped, matching what the board already shows. |
| D11 | Cycle time for a shipped feature runs from its **first cell's `trace.claimed_at`** to its **last non-dropped cell's `trace.capped_at`**. | Follows necessarily from D10: with merge no longer the completion milestone, there is no merge commit to anchor the end, so the last cap is the only timestamp that marks the feature done. |

### Agent's Discretion

Delegated to the agent, within the constraints above:

- Route shape and page layout, following mdview's existing conventions
  (`/p/:id/...` for project-scoped pages, `format!`-built HTML in `views.rs`,
  atelier CSS tokens — no new frontend framework, no build step).
- Which crate each piece lands in, given the established split: pure `.bee/`
  reading logic in `mdview-core`, routes and views in `mdview`.
- How to detect "merged into main" for D5 (git plumbing choice).
- Caching / freshness strategy for reading `.bee/` on each request.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Bee project | A project registered in `~/.mdview/registry.db` whose `root_path` contains a `.bee/` directory. Only these get the cockpit surface. |
| Shipped | Every cell of the feature is `capped` **and** its worktree is merged into main (D5). Not "capped", not "documented", not "released". |
| Cycle time | First cell's `trace.claimed_at` → the merge commit date of that feature's worktree. |
| Doing / Waiting / Stuck / Done | The four user-facing buckets over bee's five cell statuses, per D7. `dropped` maps to none of them — it is hidden. |
| Active project | A bee project with at least one cell in `open` or `claimed` (D8). Session liveness is not part of this definition. |

## Specific Ideas And References

- The user's driving question is a velocity question: "từ lúc ban đầu tới lúc hoàn
  thành toàn bộ mất bao nhiêu thời gian? 1 ngày ship được bao nhiêu? 1 tuần ship
  được bao nhiêu?" The dashboard's headline numbers must answer exactly these three.
- The user's stated pain: bee's own store is thorough but "để xem nó rất khó với
  người dùng". Legibility is the product, not completeness.
- Every panel must be clickable through to the underlying detail.

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/mdview-core/src/repository.rs:11` — `SqliteStore`; `list_projects()` at
  `:76` and the `projects` table schema at `:256-287` (`id, name, root_path,
  created_at, last_seen_at`). This is the D2 project source. `root_path` is the
  field that locates `.bee/`.
- `crates/mdview-core/src/domain.rs:6-15` — the `Project` struct.
- `crates/mdview/src/views.rs:8` — `layout()`, the shared page chrome; `:511-527`
  the `include_str!` asset bundle.
- `crates/mdview/assets/atelier/atelier.css:53-78` — the atelier colour tokens
  (`--color-bg`, `--color-surface`, `--color-text`, `--color-action`), light/dark
  aware via the `data-scheme` attribute set in `views.rs:22`.

### Established Patterns

- Server-side HTML built with `format!` in `views.rs` — no template engine (despite
  `minijinja` sitting unused in workspace deps), no `package.json`, no JS build step.
- Assets embedded at compile time via `include_str!`, served by dedicated handlers
  registered in `router()`.
- JSON endpoints return `Json(json!({...}))` from an axum handler; see
  `server.rs:145-168` (`/api/projects`) for the shape to imitate.
- Domain logic with no web dependency lives in `mdview-core`; `mdview` holds axum,
  CLI, MCP and views.

### Integration Points

- `crates/mdview/src/server.rs:92-111` — the `router()` fn; every new route
  registers here.
- `crates/mdview/src/server.rs:24-29` — `AppState`; new shared state extends this.
- `crates/mdview/src/server.rs:106` — `project_home` (`GET /p/:id/`), the natural
  place to surface an entry point to the bee view.
- `crates/mdview/src/mcp.rs:42,85` — `tools/list` and `handle_tool_call`; adding an
  MCP tool is a localized change in this one file, if planning decides one is wanted.

### Canonical References

`.bee/` schemas confirmed on disk and against the bee CLI source at
`/home/thanhsmind/projects/goglbe/beehive/packages/bee-rs/crates/bee/src/`:

- `.bee/state.json` — `{schema_version, phase, feature, mode, approved_gates{context,shape,execution,review}, workers[], summary, next_action}`.
- `.bee/cells/<id>.json`, archived at `.bee/cells/archive/<feature>/<id>.json` —
  `{id, feature, title, action, verify, files[], read_first[], deps[], decisions[], must_haves{}, behavior_change, pbi, change_class, lane, status, tier, trace{}}`.
  - `status` ∈ `open | claimed | blocked | capped | dropped`. Only `open` and
    `claimed` are schedulable; `capped` and `dropped` are terminal.
  - `lane` ∈ `tiny | small | standard | high-risk | spike`.
  - `trace` accumulates `worker, claim_session, claimed_at, fix_first` on claim;
    `capped_at, outcome, files_changed, deviations, tests, results, ran_at` on cap;
    `blocked_reason`, `dropped_reason`, `reopened_at`, `reopened_reason` on the
    other transitions. **These timestamps are the only true cycle-time source.**
- `.bee/sessions/<uuid>.json` — `{id, started_at, last_heartbeat, transcript_path, workspace_id, source}`. `last_heartbeat` is the liveness signal.
- `.bee/backlog.jsonl` — two row shapes: finding rows `{ts, type, title, detail, severity, layer, feature}` with `severity ∈ P1|P2|P3`, and event-sourced PBI rows `{kind:"pbi", id, title, status, cos, feature}` with `status ∈ proposed|in-flight|parked|done|declined`, folded to current state.
- `.bee/decisions.jsonl` — event-sourced; `type ∈ decide|tag|redact|supersede|stub`. A `decide` row is `{id, type, date, decision, rationale, alternatives, scope, source, confidence, tags[]}`. Archive at `.bee/decisions-archive.jsonl`.
- `.bee/runtime/workspaces/<id>.json` — `{id, type, root, branch, base_sha, write_owner_session, fence_epoch, attached_sessions, created_at}`. This is the worktree/branch link D5 needs.
- `.bee/reservations.json` — `{reservations: [{agent, cell, path, expires, session, kind}]}`.
- `.bee/logs/timings.jsonl` — `{ts, cmd, ms, ok}`; `dispatch.jsonl` — per-agent-dispatch records incl. `tier`, `effective_model`.
- `docs/history/<feature>/plan.md` — YAML frontmatter carries `artifact_contract: bee-plan/v1`, `mode`, and `approved_gate2: <date>`. The only machine-readable field in the docs tree.
- `.bee/config.json` — `{hooks{6 bools}, gate_bypass, models{claude{extraction,generation}, codex{...}}}`.

Registry state at shaping time: 5 projects registered in `~/.mdview/registry.db`
(beehive, anphabe-gogl, anphabe-bi-dashboard, a-blog, vnbptw-mapcompany) — all 5
contain `.bee/`.

## Outstanding Questions

### Resolve Before Planning

None. All three product questions raised during shaping are resolved as D7, D8, D9.

### Resolve Before Slice 2 — RESOLVED 2026-08-05 by D10 and D11

Both questions below are answered. The user's call: a feature with no worktree but all
cells capped **is** shipped, and a dropped cell does **not** block shipped status. That
is D10; D11 follows for cycle time. The original questions are kept for the record.

- [ ] **D5 marks main-lane work as never shipped.** Measured on beehive: of the 25
  features present in its live cells, 8 have no `wt/<slug>` merge commit —
  `doctrine-prose-diet`, `exec-speed`, `release-2-1-7`, `release-2-1-8`,
  `review-p1-fixes`, `session-capture`, `windows-shortpath`, `workflow-lifecycle`.
  These are releases and docs-lane work, which AGENTS.md explicitly permits to land in
  the main checkout without a worktree. Under D5 as written they can never count as
  shipped, so "1 ngày ship được bao nhiêu" undercounts by roughly a third. Options:
  (a) treat a feature whose cells are all capped as shipped when it never had a
  worktree, (b) restrict the velocity number to worktree-lane features and label it as
  such, (c) keep D5 and show the gap as an explicit "unmerged" bucket.
- [ ] **D5 and D7 disagree about dropped cells.** D5 requires *every* cell capped; D7
  hides `dropped`. beehive's `dispatch-worktree` is capped-plus-dropped and merged
  (`15517df7`), and beehive holds 7 dropped cells across 25 features. As written, the
  board would show every visible cell of that feature as Done while the velocity number
  says it never shipped. Options: a dropped cell (a) does not block shipped-ness, or
  (b) does, and the board grows a marker explaining why a Done-looking feature is not
  counted.

### Deferred To Planning

- [ ] How to determine "merged into main" for D5 — git plumbing over the worktree
  branch recorded in `.bee/runtime/workspaces/*.json`, versus a marker bee already
  writes at `bee worktree merge`. Investigation: read the merge verb in the bee CLI
  source and check what it records.
- [ ] Whether historical features predating the current bee version have enough
  trace data for cycle time, and what the page shows when they do not.
- [ ] Read cost: whether `.bee/` is parsed per request or cached, given `.bee/logs/*.jsonl`
  grow unbounded and a project may hold hundreds of archived cells.
- [ ] Whether the bee surface needs its own MCP tool, or is web-only in phase 1.
- [ ] `server.rs:157-160` deliberately omits `root_path` from `/api/projects` to avoid
  leaking filesystem paths on a non-loopback bind. Bee data is path-shaped throughout
  (cell `files[]`, reservation `path`, session `transcript_path`, workspace `root`).
  What the JSON surface may expose needs the same treatment.

## Deferred Ideas

Out-of-scope ideas captured during shaping. Not lost, not planned.

- **Cross-project roll-up page (phase 2).** One page over every registered project:
  active project count, ship velocity across all of them, which lanes are running
  where, a combined session view. This was the user's original framing ("bee đang
  hoạt động trên những dự án nào?"); D6 narrows phase 1 to the per-project view first.
  Recorded here rather than in `.bee/backlog.jsonl` — `bee backlog add` rejected every
  `--type` value tried and the accepted kind table is not present in the reachable
  bee source.
- **A global `~/.config/bee` store.** The user's opening idea. D2 supersedes it for
  phase 1: mdview's registry already answers "which projects", so no second registry
  is built. Revisit only if phase 2 needs cross-project data that cannot be assembled
  by reading each project's own `.bee/` on demand.
- **Writing from the dashboard** — approving a gate, claiming a cell, parking a
  backlog item. D4 rules it out for this feature.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.

The three questions under **Resolve Before Planning** are product decisions the user
owns — they change what the page's numbers mean, not how it is built.
