promote proposal for work item "backlog-groom-1" (.bee/logs/scribing-runs.jsonl + .bee/lanes/backlog-groom-1.json) — 2 capped cell(s): backlog-groom-1-1, backlog-groom-1-2
anchor: ledger — .bee/logs/scribing-runs.jsonl, .bee/lanes/backlog-groom-1.json
PROPOSAL ONLY — nothing was written. Applying any section below is a human or agent decision.

(a) DELIVERY DRAFT — save as docs/knowledge/work/backlog-groom-1/delivery.md

---
type: bee.delivery
title: backlog-groom-1 — delivery
description: "Delivery record proposed by bee knowledge promote for work item backlog-groom-1: 2 capped cell(s), 0 recorded deviation(s)."
timestamp: 2026-08-16
bee:
  id: backlog-groom-1-delivery
  lifecycle: active
  areas: [system-overview]
  required_context: [.bee/logs/scribing-runs.jsonl, .bee/lanes/backlog-groom-1.json]
  sources: [.bee/logs/scribing-runs.jsonl, .bee/lanes/backlog-groom-1.json, .bee/cells/backlog-groom-1-1.json, .bee/cells/backlog-groom-1-2.json]
---

# backlog-groom-1 — Delivery

## What shipped

- **backlog-groom-1-1** — Added unregister_project_removes_a_registered_project_from_the_registry proving Engine::unregister's happy path (registered project disappears from the registry after a same-origin POST). The too_slow and failed register-scan codes (finding #12) are untestable at the route level without a production seam: REGISTER_SCAN_BUDGET is a hardcoded 2s const with no override knob (a real trigger needed ~2.6M+ fs entries per a 200k-file/0.155s local benchmark, impractical/flaky), and no store/scan error is reachable through the crate's public API (rusqlite is not a waggledance-crate dependency, SqliteStore exposes no fault-injection hook, ON CONFLICT upsert avoids constraint errors, and fs errors are swallowed to None throughout indexer.rs) short of a ~9999-registration unique_id-exhaustion trick that would not match the finding's named trigger. Noted rather than adding a production hook. cargo test --workspace: 1026 passed; cargo fmt --check and cargo clippy --workspace --all-targets -- -D warnings both clean. (1 file(s) changed)
- **backlog-groom-1-2** — Added SqliteStore::file_content (plain read of files_fts.content); Engine::index_file_incremental now returns Result<bool> comparing new vs stored content before the write (brand-new path = changed); watch.rs reindex_paths broadcasts the WS reload only when that signal is true. cargo test --workspace (1030 passed), cargo fmt --check, and cargo clippy --all-targets -D warnings all green. (3 file(s) changed)

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **backlog-groom-1-1** — `cargo test --workspace green with the new tests present and passing; the too_slow and failed codes and the unregister-removes-the-project behavior are each asserted by a named test.`
- **backlog-groom-1-2** — `cargo test --workspace green (CI triple: fmt + clippy + test). New tests: a watch/index test where reindexing byte-identical content emits no reload while changed content and a brand-new path each do; a repository/engine unit test that index_file_incremental returns not-changed for identical content and changed for differing content. Update existing callers/tests of index_file_incremental to the new signature.`

## Deviations

None recorded in the capped cell traces.

## Provenance

Proposed by `bee knowledge promote --work backlog-groom-1` from 2 capped cell trace(s) in `.bee/cells/` and the anchor `.bee/logs/scribing-runs.jsonl`, `.bee/lanes/backlog-groom-1.json`. Every line above is copied from a trace or from the work item; nothing here is curated truth until a human or agent accepts it.

(b) AREA UPDATES — candidate spec-sync bullets, each citing its cell

areas: from the scribing stamp for "backlog-groom-1" — .bee/logs/scribing-runs.jsonl's most recent entry (2026-08-16T03:04:37.190Z), the work item declares no bee.areas.

area system-overview:
  - [backlog-groom-1-2] Added SqliteStore::file_content (plain read of files_fts.content); Engine::index_file_incremental now returns Result<bool> comparing new vs stored content before the write (brand-new path = changed); watch.rs reindex_paths broadcasts the WS reload only when that signal is true. cargo test --workspace (1030 passed), cargo fmt --check, and cargo clippy --all-targets -D warnings all green. — feature-wide sync per the scribing stamp, 3 file(s) changed (trace .bee/cells/backlog-groom-1-2.json)

(c) PATTERN CANDIDATES — candidate bee.pattern concepts, bee.polarity pitfall

None: no capped cell trace carries a deviation or a failure signature.

knowledge promote: 2 capped cell(s) mined, 1 delivery draft, 1 area bullet(s), 0 pattern candidate(s), 0 file(s) written.