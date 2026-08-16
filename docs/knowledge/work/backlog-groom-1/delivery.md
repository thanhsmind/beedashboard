---
type: bee.delivery
title: backlog-groom-1 — delivery
description: "Delivery record for work item backlog-groom-1: the unregister happy-path pinned by a test, and the live watcher's reload broadcast gated on content actually changing."
timestamp: 2026-08-16
bee:
  id: backlog-groom-1-delivery
  lifecycle: active
  areas: [system-overview]
  required_context: [docs/specs/system-overview.md]
  sources: [.bee/lanes/backlog-groom-1.json, .bee/cells/backlog-groom-1-1.json, .bee/cells/backlog-groom-1-2.json]
---

# backlog-groom-1 — Delivery

## What shipped

- **backlog-groom-1-1** — Added unregister_project_removes_a_registered_project_from_the_registry proving Engine::unregister's happy path (registered project disappears from the registry after a same-origin POST). The too_slow and failed register-scan codes (finding #12) are untestable at the route level without a production seam: REGISTER_SCAN_BUDGET is a hardcoded 2s const with no override knob (a real trigger needed ~2.6M+ fs entries per a 200k-file/0.155s local benchmark, impractical/flaky), and no store/scan error is reachable through the crate's public API (rusqlite is not a waggledance-crate dependency, SqliteStore exposes no fault-injection hook, ON CONFLICT upsert avoids constraint errors, and fs errors are swallowed to None throughout indexer.rs) short of a ~9999-registration unique_id-exhaustion trick that would not match the finding's named trigger. Noted rather than adding a production hook. cargo test --workspace: 1026 passed; cargo fmt --check and cargo clippy --workspace --all-targets -- -D warnings both clean. (1 file(s) changed)
- **backlog-groom-1-2** — Added SqliteStore::file_content (plain read of files_fts.content); Engine::index_file_incremental now returns Result<bool> comparing new vs stored content before the write (brand-new path = changed); watch.rs reindex_paths broadcasts the WS reload only when that signal is true. cargo test --workspace (1030 passed), cargo fmt --check, and cargo clippy --all-targets -D warnings all green. (3 file(s) changed)

## Verify

- **backlog-groom-1-1** — `cargo test --workspace` green with the new named tests present and passing.
- **backlog-groom-1-2** — CI triple green; new tests prove byte-identical reindexing emits no reload while changed content and a brand-new path each do, and index_file_incremental reports changed/not-changed correctly.

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work backlog-groom-1` from 2 capped cell traces. Accepted at the compounding pass on 2026-08-16. The proposal's one system-overview bullet was checked against the living spec and found already merged by the feature's own scribing sync: `docs/specs/system-overview.md` states the reload "signal fires only when a reindexed file's content actually changed". No pattern candidates were proposed.
