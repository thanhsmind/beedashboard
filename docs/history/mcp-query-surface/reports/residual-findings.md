# mcp-query-surface — residual review findings (P3)

Review session: `review-2026-08-16-mcp-query-surface` (scope 991923b..229a542).
Non-blocking; each either becomes a backlog row or is accepted with reason at review close.

1. **Store-error masking in project lookup** — `mcp.rs:235,328`: `get_project(...).ok().flatten()` reports a DB failure as `no such project`, sending the agent to re-register a registered project. Fix: match the Err arm, name the real fault.
2. **file_count error indistinguishable from empty** — `mcp.rs:298`: `unwrap_or(0)` renders a store error as 0. Fix: null or a sibling error field.
3. **O(n²) rel_path lookup in refresh_stale** — `engine.rs:216`: linear `existing.iter().find` per walked file; largest live project 1802 rows. Fix: one HashMap before the loop.
4. **Snippet 12→64 blast radius incompletely recorded** — `repository.rs:310` is the single shared snippet call; plan recorded the web card growth, not the CLI (`cli.rs:395` prints untruncated) — and neither surface clamps.
5. **Non-alphanumeric query answers "No hits"** — `mcp.rs:218` + `repository.rs:496-503`: "???" sanitizes to empty and short-circuits; "C++" searches `"C"*`. Agent can't tell "no match" from "query discarded".
6. **Unreadable-path skip untested** — `engine.rs:200-203`: documented skip behavior (broken symlink/permission) has no covering test.
7. **Deleted file → search absence never asserted end-to-end** — `engine.rs:733` asserts row absence; FTS absence only via a pre-existing store test. One search assertion closes it.
8. **Filtered-branch freshness untested** — `mcp.rs:240`: the `Some(project)` refresh arm passes tests identically if the refresh call is deleted; only the unfiltered arm has a freshness test.
9. **Filtered ask_state absent-.bee branch untested** — `mcp.rs:333-335`: only the rollup path covers a project without `.bee/`.
10. **Rollup zip length invariant unpinned** — `mcp.rs:366`: labeling by `zip` is safe only while `read_rollup` stays 1:1 with roots (`bee.rs:1269-1277`); no test pins it.
11. **view_file routing through the new match untested** — `mcp.rs:151`: three new arms and the fallthrough are tested; the refactored view_file arm is not (existing tests sit below the dispatcher).
12. **busy_timeout is process-wide** — `repository.rs:36-37`: the 5s pragma applies to the daemon too, whose async handlers call the store without spawn_blocking — a contended op now stalls a tokio worker up to 5s while holding the connection mutex. Related: MCP-driven re-index emits no viewer reload signal (`engine.rs:223` skips index_file_incremental's changed flag).
