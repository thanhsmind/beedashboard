# MCP Query Surface — Context

**Feature slug:** mcp-query-surface
**Date:** 2026-08-16
**Shaping session:** complete
**Scope:** Standard
**Domain types:** CALL | READ

## Feature Boundary

Waggledance's MCP server grows from one write-side tool (`waggledance_view_file`)
to an agent-facing query surface: agents ask waggledance for cross-project
document content and parsed bee state and get structured, ready-to-use answers —
ending the write-code-then-re-read-files loop. The web UI, the CLI, and the
existing `waggledance_view_file` tool are unchanged.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | Query tools default to ALL registered projects; an optional project filter narrows the scope. (bee decision 4a47e125) | Waggledance is the cross-project info holder; an agent in one repo benefits from hits in sibling repos. |
| D2 | Search hits return a path anchor plus a rich snippet — enough surrounding context to answer most questions without a follow-up Read. Never bare path lists; never whole files by default. (bee decision 4bf9bacb) | The re-read loop is the complaint this feature kills; bare paths keep it, whole files waste tokens. |
| D3 | v1 ships three tools together: `waggledance_search` (FTS5 full-text), `waggledance_projects` (registry + index status), `waggledance_ask_state` (parsed bee state: active feature, locked decisions, open cells). No slice split. (bee decision 30265f91) | bee.rs already parses everything; marginal cost is tool schema. |
| D4 | A query re-indexes changed files in the touched project(s) before returning. Search never serves stale results for files modified on disk but not yet indexed. (bee decision 1b83d13c) | The info holder must reflect current disk; slight latency accepted over stale answers. |

### Agent's Discretion

Snippet sizing, FTS ranking, tool input/output JSON schemas, and how the
pre-query re-index scan is bounded (e.g. mtime walk vs watcher state) are
planning/implementation choices — constrained only by D2 (snippet must be
answer-sufficient, not whole-file) and D4 (no stale results).

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| query surface | The set of MCP tools an agent calls to ask waggledance questions, as opposed to the view/write surface (`waggledance_view_file`). |
| rich snippet | An excerpt around the match large enough to answer the question in-place; more than a title/line, less than the whole file. |
| stale result | A hit whose indexed content differs from the file's current bytes on disk at query time. |

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance-core/src/repository.rs` — SQLite registry + file index + FTS5 (`search()` at line 294, `files_fts` virtual table, FTS-safe query quoting at ~491). The search engine already exists; v1 exposes it.
- `crates/waggledance-core/src/bee.rs` — parses bee state (sessions, cells, decisions, knowledge) for the dashboard; `waggledance_ask_state` reads from the same parsed model.
- `crates/waggledance-core/src/indexer.rs` — existing indexing path; D4's pre-query re-index reuses it.

### Integration Points

- `crates/waggledance/src/mcp.rs` — hand-rolled MCP server currently exposing exactly one tool (`tool_schema()` line 58, dispatch guard line 86). New tools register and dispatch here.
- Web `_search` route (`crates/waggledance/src/views.rs` ~5903) — human-facing consumer of the same FTS; must keep working unchanged.

## Canonical References

- `docs/backlog.md` — PBI p-097cf752 (this feature's origin).
- PRD §5.5 — the existing MCP contract for `waggledance_view_file`.

## Outstanding Questions

### Deferred To Planning

- [ ] How the pre-query re-index is bounded (full mtime walk vs incremental) — measure against a registry the size of the current one; D4 fixes the guarantee, not the mechanism.
- [ ] Exact `ask_state` answer vocabulary (which questions map to which state slices) — derive from what bee.rs already models.

## Deferred Ideas

Out-of-scope ideas captured during shaping. Not lost, not planned.

- `waggledance_context` — a digest tool that synthesizes an answer to a free-form question across hits, instead of returning hits. Deferred: v1 proves the raw query surface first.

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
