---
artifact_contract: bee-plan/v1
mode: standard
# approved_gate2: <unset until approval>
---

# Plan: Bee Cockpit

Mode: `standard` — 3 risk flags: audit-security (bee data is path-shaped and mdview's
server has no auth), public-contracts (new HTTP routes + JSON shapes), multi-domain
(bee-state parsing + web surface + git).

Why this is the least workflow that protects the work: it is a read-only view over
files that already exist, so nothing can be corrupted — but it puts absolute
filesystem paths one route away from an unauthenticated server, and that single
constraint is worth a written shape and a gate.

**Deviation, stated:** the flag `audit-security` is on the hard-gate list, which would
route this to `high-risk`. Routed `standard` instead: the exposure is one bounded rule
(never emit an absolute path), the same class of concern `server.rs:157-160` already
solves in four lines for `/api/projects`, and there is in-repo precedent to copy. If
the review wave finds the path surface is wider than that, re-route up.

## Requirements (from CONTEXT.md)

- **D1** — Build inside the existing mdview Rust workspace. mdview source stays.
- **D2** — Projects come from mdview's registry (`~/.mdview/registry.db`, `projects`
  table). No new `~/.config/bee` registry.
- **D3** — A registered project shows the bee surface only when its `root_path`
  contains `.bee/`. Without it, behavior is exactly as today — no tab, no empty panel.
- **D4** — Strictly read-only. No gate approval, no cell claim, no state write.
- **D5** — Shipped = every cell of the feature `capped` **and** its worktree merged
  into main. Cycle time = first cell's `trace.claimed_at` → merge commit date.
- **D6** — Phase 1 is the per-project view only. Cross-project roll-up is later.
- **D7** — Four buckets: Doing (`claimed`), Waiting (`open`), Stuck (`blocked`, red,
  its own bucket), Done (`capped`). `dropped` hidden.
- **D8** — A project is *active* when it holds ≥1 cell in `open` or `claimed`.
- **D9** — Read live `.bee/cells/*.json` only; `.bee/cells/archive/` is not read.

## Discovery

- **Registry is real and populated — with empty stores in it.** `sqlite3
  ~/.mdview/registry.db` → **6** projects; all 6 contain `.bee/`. But two of them
  (a-blog, and beedashboard itself, self-registered) hold **zero** cells. An empty
  `.bee/cells/` is therefore the common case, not an edge case, and Slice 1 must render
  it as a real state. Schema `projects(id, name, root_path, created_at, last_seen_at)`
  at `crates/mdview-core/src/repository.rs:256-287`; `list_projects()` at `:76`.
- **Live-cell counts (verified):** beehive 90, anphabe-gogl 68, vnbptw-mapcompany 27,
  anphabe-bi-dashboard 15, a-blog 0, beedashboard 0 — **200 live vs 5 archived**, which
  is why D9 costs almost nothing. `trace.claimed_at` / `trace.capped_at` carry the
  timestamps; nothing else does.
- **Merge detection works mechanically but does NOT cover most features — see Known
  Limit below.** `bee worktree merge` runs
  `git merge --no-ff --no-commit -- wt/<slug>` and commits
  `"Merge worktree <id> (branch wt/<slug>) via bee worktree merge"`
  (`beehive/packages/bee-rs/crates/bee/src/verbs/worktree/phases.rs:184,186`). Measured
  on beehive: of the 25 features present in its live cells, **17 have such a merge
  commit and 8 do not** — releases and docs-lane work land in the main checkout, which
  AGENTS.md explicitly permits. Main's history alone would mark ~32% of genuinely
  shipped features as never-shipped.
- **The path-leak precedent is in-repo.** `crates/mdview/src/server.rs:157-160`
  deliberately omits `root_path` from `/api/projects` — *"the server has no
  authentication, so exposing each project's filesystem layout over `/api/projects`
  leaks it to anyone who can reach the port"* — and `:69-77` warns on non-loopback
  binds. This is the pattern to copy, not invent.
- **No HTTP test harness exists yet.** `crates/mdview/Cargo.toml` has **no
  `[dev-dependencies]` section at all`**; `server.rs` holds two inline `#[cfg(test)]`
  modules (`:589` `highlight_css_tests`, `:610` `asset_response_tests`, file ends `:705`)
  and both are pure-function — zero async tests, nothing drives `router()`. Slice 1 must
  add the harness, or its route-level tests have no teeth.
- **Baseline is green.** `cargo test --workspace` → exit 0, 165s
  (`.bee/logs/test-results.json`, 2026-08-05T03:16:02Z). `commands.test` was undeclared
  before this plan and is now declared, so cells can cap. (`commands.verify` was briefly
  added and removed — bee retired that key in 2.1.0 and ignores it.)

### Known Limit — D5 does not cover main-lane work

D5 defines shipped as *all cells capped **and** the worktree merged into main*. Measured
above, 8 of beehive's 25 live-cell features never had a worktree, so under D5 as written
they can never be shipped, and the headline "features/day" undercounts by roughly a third.

A second gap: D5 requires *every* cell capped, while D7 hides `dropped` cells. beehive's
`dispatch-worktree` is capped-plus-dropped **and** merged (`15517df7`). The board would
show every visible cell Done while the velocity number says it never shipped — two panels
contradicting each other, neither wrong by the letter of the decisions.

Both are real and both are the user's to resolve, not planning's to narrow. Neither blocks
Slice 1, which does not compute shipped-ness. **They must be resolved before Slice 2
starts** — recorded as Outstanding Questions in CONTEXT.md.

## Approach

**Recommended.** A pure `bee` reader module in `mdview-core` (no web dependency, per
the crate split at `crates/mdview-core/Cargo.toml:9-26`) that turns a project root into
a typed snapshot, plus routes and hand-written `format!` views in `crates/mdview`,
following `views.rs` and `server.rs:92-111` exactly. Per D2 the project list is
`SqliteStore::list_projects()`; per D3 the surface is gated on `.bee/` existing; per
D4 every code path is a read.

**Rejected — a background indexer that mirrors `.bee/` into the SQLite registry.**
Faster reads, but it duplicates state that live bee sessions mutate underneath us, and
a stale mirror showing a wrong cell status is worse than a slow correct page. Read on
request instead.

**Rejected — a JS frontend for the drill-downs.** The repo has no `package.json` and
no build step; every page is server-rendered `format!` HTML with `include_str!` assets.
Adding a toolchain for one surface costs more than it returns.

**Rejected — extending `bee` itself with an aggregation verb.** Would put the
dashboard's needs inside a different repo's release cycle. D1 already placed this in
mdview.

**Risk map.**

| Component | Risk | Proof needed |
|---|---|---|
| Path disclosure in HTML + JSON | **HIGH** | A test asserting no absolute path (no `/home/`, no drive-letter prefix) appears in the rendered bee page or its JSON, for a fixture project whose cells carry absolute `files[]`. |
| `.bee/` parsing against real-world drift | MEDIUM | Reader tolerates a missing file, an unknown `status` value, and a malformed JSON line without failing the page. Proven on fixtures, then spot-checked against the 5 registered projects. |
| Read cost on a big store (beehive: 90 live cells, unbounded `logs/*.jsonl`) | MEDIUM | Slice 1 reads `state.json` + `cells/*.json` only, never `logs/`. Timed once against beehive. |
| Read-only-ness (D4) silently broken by a cache write | MEDIUM | A test that snapshots the fixture's whole `.bee/` tree before a request and asserts it is byte-identical after. Without it, a cache landing in `.bee/cache/` would keep the suite green and race a live session. |
| Route/view integration | LOW | Requires a harness that does not exist yet — `crates/mdview/Cargo.toml` has no `[dev-dependencies]`. Slice 1 adds `tower` + `http-body-util` as dev-deps and drives `router()` with `oneshot`; without it the route tests degrade to view-function assertions and D3's "no empty panel" half goes unproven. |

## Shape

Numbered **Slice 1/2/3** throughout, never "phase" — CONTEXT.md already uses "phase 2"
for the cross-project roll-up (D6), and a cold worker handed "Phase 2" would build the
wrong thing.

| Slice | What Changes | Why Now | Demo | Unlocks |
|---|---|---|---|---|
| **1 — Cell board (current slice)** | New `mdview-core` bee reader (presence, `state.json`, live `cells/*.json`) producing the four D7 buckets and the D8 active flag; the `[dev-dependencies]` harness that drives `router()`; new `GET /p/:id/_bee` page; conditional entry point from project home per D3 | This is the user's most-asked question — "cell nào đang làm, cái nào đã xong, cái nào đang đợi" — and it is the walking skeleton: real data, real route, real page, no stubs | Open `/p/beehive/_bee` and see beehive's 90 real cells sorted into Doing / Waiting / Stuck / Done; open `/p/a-blog/_bee` and see the empty state | Every later panel hangs off the same reader and page |
| **2 — Ship velocity** | Group cells by `feature`; mark shipped per D5 as amended by the two Outstanding Questions; derive features/day, features/week, and cycle time from `trace.claimed_at` → merge date | Answers the question that started this: "1 ngày ship được bao nhiêu, 1 tuần bao nhiêu" | The three headline numbers, computed from beehive's real history | The cross-project roll-up (D6, deferred) |
| **3 — The rest of the store** | Backlog panel (PBI fold + findings), sessions/lanes panel, decisions, and click-through detail for every cell, feature, session and backlog row | The user asked to see all of it, and drill-down is the stated point of the page | Click any cell in the board and read its full `trace` | Nothing further in this feature |

**Current slice to prepare: Slice 1.**
**Slice 2 is blocked** until the two D5 Outstanding Questions are answered.

**Smaller path check.** Could Slice 1 be smaller and still honor its decisions?
Considered shipping the reader alone with a JSON endpoint and no HTML — rejected: D3 and
D7 are both about what a *person* sees, and a JSON-only slice proves neither. Considered
folding Slice 2 in — rejected: D5 needs git plumbing and two unanswered questions, and the
board is useful without it. Considered skipping the dev-dependency harness and testing the
view function directly — rejected: that is exactly how D3's "no empty panel" half goes
unproven (review finding). Slice 1 as scoped is the smallest slice that demonstrates
D1, D2, D3, D4, D7, D8 and D9 end to end. It does **not** demonstrate D5; that is Slice 2's
job and is stated here rather than glossed.

## Test matrix

The triad, at its smallest demonstrating size. Each cell's writer judges existing
coverage first (`server.rs:589-706`, `repository.rs:321-408` already pin the
neighbouring behavior) and authors only the gap.

**Happy path**
- A fixture project root with `.bee/state.json` and cells across all five statuses
  produces the four D7 buckets with the right counts, and `dropped` appears in none.
- A project with ≥1 `open` or `claimed` cell reports active per D8; one with only
  `capped`/`dropped` cells does not.
- `GET /p/:id/_bee` returns 200 and the rendered HTML contains each bucket's count.

**Edge cases**
- `.bee/` absent → **both halves of D3**: no bee entry point on the project home page,
  *and* `GET /p/:id/_bee` returns a clean not-found rather than rendering an empty bee
  page. The second half requires driving `router()` through the new harness; a
  view-function assertion cannot prove it.
- `.bee/cells/` present but empty → the page renders four zero buckets, not an error.
  This is 2 of the 6 registered projects today, not a hypothetical.
- A cell carrying an unknown `status` string is counted in no bucket and does not
  abort the page.
- `.bee/cells/archive/` populated → those cells are absent from every count (D9).

**Error paths**
- A malformed `.bee/state.json` and a truncated `cells/<id>.json` each degrade to a
  partial snapshot; the page still renders and names what could not be read.
- **Security probe (the HIGH risk above):** a fixture cell whose `files[]`,
  `trace.worker` and session `transcript_path` hold absolute paths. Assert the rendered
  page and any JSON response contain **no occurrence of the fixture's own root path**,
  and no `std::path::is_absolute()` path anywhere in the emitted strings. Do *not*
  assert on the literal `/home/` — fixtures build under `std::env::temp_dir()`
  (`crates/mdview/src/runtime.rs:249`), so a `/home/`-only check passes green while the
  page leaks `/tmp/...` verbatim. That toothless form was the review's finding.
- **Read-only probe (D4):** snapshot every file under the fixture's `.bee/` (paths +
  contents hash) before a request to `/p/:id/_bee`, assert byte-identical after. Fails
  if any caching or bookkeeping write lands in the project's own store.

## Out of scope

- The cross-project roll-up over all registered projects (D6; deferred idea in
  CONTEXT.md).
- A global `~/.config/bee` store (superseded by D2).
- Any write action — gate approval, cell claim, backlog edit, session kill (D4).
- Reading `.bee/cells/archive/` (D9), `.bee/logs/*.jsonl`, and the `.bee/expertise/`
  markdown tree.
- An MCP tool for the bee surface — web-only in this feature.
