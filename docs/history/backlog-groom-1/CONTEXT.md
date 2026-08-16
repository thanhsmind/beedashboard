# Backlog Groom 1 — Context (batch bugfix)

**Feature slug:** backlog-groom-1 · 2026-08-16 · standard · bugfix batch

## Scope

Three bounded review findings on non-contended files (server.rs, watch.rs).
`views.rs` findings (#26, #8, #32, #34) are deferred — another session holds
views.rs. Each cell runs sequentially (one worker at a time in this one worktree).

## Locked Decisions

| ID | Decision | Rationale |
|----|----------|-----------|
| D1 | The register scan error codes `too_slow` and `failed` each get a test asserting the code is returned (finding #12); `Engine::unregister` and the `POST …/unregister` happy path each get a test proving a registered project actually disappears (finding #9). Tests only — no behavior change. | Live write-endpoints and scan error paths shipped unproven. |
| D2 | The live-reload watcher only broadcasts a reload for a path whose content actually changed: `reindex_paths` (watch.rs) compares new content against stored and skips the reload signal when identical. | Decision log 18228765. A touch / no-op rewrite currently reloads every client (finding #19). |

### Agent's Discretion
- Test fixtures/harness reuse for D1 — follow the existing server.rs test patterns.
- D2's identity check: content hash vs stored-blob compare — whichever the indexer already computes; do not add a second hashing pass if one exists.

## Existing Code Context
- `crates/waggledance/src/server.rs` — register scan codes at ~:1442 (too_slow), :1356/:1426 (failed); `Engine::unregister` at waggledance-core/src/engine.rs:123; unregister route + the Host-header test at ~server.rs:22870.
- `crates/waggledance/src/watch.rs` — `reindex_paths` ~:52-84 reports every reindexed path as changed.

## Verify
`cargo test --workspace` green (declared gate = CI triple: fmt + clippy + test).
D1 is proven by its own new tests; D2 by a watch test that a no-content-change
reindex emits no reload plus a real change still does.

## Handoff
Cells run sequentially in one worktree. CONTEXT is source of truth.
