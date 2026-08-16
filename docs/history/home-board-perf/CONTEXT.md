# Home Board Perf — Context

**Feature slug:** home-board-perf · 2026-08-16 · standard · bugfix
**Scope:** #28 from review-2026-08-16.

## Problem (verified)

- `read_snapshot(root)` (crates/waggledance-core/src/bee.rs:1009) walks all of a
  project's `.bee/**` plus `docs/history/<feature>/CONTEXT.md` for every feature it
  discovers. No cache. The home page (`index_page`, server.rs:717) calls it once per
  registered project via `cross_project_rollup` (spawn_blocking per project), on every
  `/` render — O(projects × features) filesystem work per page load.
- The home board reloads on ANY changed markdown in ANY project: `shouldReload` at `/`
  (app.js:860-876) has no matching path, so `if (!m) return true` fires for every
  `{"changed":[...]}` broadcast — a README edit in any project force-reloads the board
  though it cannot change it. (Note: `.bee/*.json` changes never reload — dropped by the
  watcher's `is_markdown` filter — that under-broad half is a separate deferred item.)

## Locked Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D1 | Cache `read_snapshot`'s result per project behind a cheap **fingerprint** of the project's `.bee/` tree plus its `docs/history/` tree — a stat-only recursive walk collecting (max mtime, entry count) over those trees, no JSON parse. A request computes the fingerprint; on a match it reuses the cached `BeeSnapshot`; on any change (a file added, removed, or modified bumps mtime or count) it re-reads and re-caches. | Decision a8710565. The stat walk is far cheaper than the full parse; the home board becomes O(cheap) on repeat renders. Entry-count in the key catches add/remove that mtime alone can miss. |
| D2 | The home board (`/`) reloads on a watcher change only when a changed path is **board-relevant** — under `docs/history/` (the only markdown `read_snapshot` depends on) — not on arbitrary project markdown. board-relevance is ONE shared predicate used by the reload decision. | Decision 57a5f387. Scopes the reload storm to the board's actual markdown dependency. |

### Agent's Discretion
- D1: where the cache lives (a `Mutex<HashMap<PathBuf, (Fingerprint, BeeSnapshot)>>` in `AppState`/server, or a wrapper in waggledance-core) — prefer the server layer so `read_snapshot` stays pure. The fingerprint walk must itself be cheap: stat only, skip `.bee/cells/archive/` deep contents if the top-level listing already reflects add/remove.
- D2: implement the board-relevance test in `shouldReload` (client) keyed on the `changed` paths (each is `<project_id>/<repo-rel-path>`); a path counts as board-relevant when its repo-rel part starts with `docs/history/`. Keep the `.term-screen` guard and every existing per-project-page rule exactly as now.

## Deferred (filed separately)
- Making a `.bee/*.json` change itself trigger a live board reload (today it never does) — a watcher/`is_markdown` change; larger, separate enhancement.

## Existing Code Context (HEAD anchors)
- read_snapshot: bee.rs:1009; read_rollup: bee.rs:1259; call from home: server.rs:889-906 (cross_project_rollup), index_page server.rs:717.
- Reload: watch.rs broadcast `{"changed":[...]}` (~watch.rs:30), each entry `<project_id>/<rel>`; app.js shouldReload 860-876.

## Verify
`cargo test --workspace` green (CI triple). D1: a Rust test that two consecutive
read-through-cache calls with no filesystem change return without re-walking (e.g. a
cache-hit counter or a snapshot identity), and that a change to a `.bee` file or a
`docs/history` file invalidates. D2: a JS-only guard recorded per home-terminal-header-2
plus, if feasible, a pure-function test of the board-relevance predicate — a `changed`
list of only non-`docs/history` paths does not reload `/`, one with a `docs/history` path
does.

## Handoff
Two cells, sequential in one worktree: D1 cache first, then D2 reload scope.
