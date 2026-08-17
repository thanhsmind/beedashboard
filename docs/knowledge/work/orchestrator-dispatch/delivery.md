---
type: bee.delivery
title: orchestrator-dispatch — delivery
description: "Delivery record for work item orchestrator-dispatch: 4 capped cells shipping the MCP orchestration surface (dispatch/await/runs), the fail-closed send/wait protocol, durable run state, and the read-only Runs view."
timestamp: 2026-08-16
bee:
  id: orchestrator-dispatch-delivery
  lifecycle: active
  areas: [agent-terminal]
  required_context: [docs/history/orchestrator-dispatch/CONTEXT.md, docs/history/orchestrator-dispatch/plan.md]
  sources: [docs/history/orchestrator-dispatch/CONTEXT.md, docs/history/orchestrator-dispatch/plan.md, .bee/cells/orchestrator-dispatch-1.json, .bee/cells/orchestrator-dispatch-2.json, .bee/cells/orchestrator-dispatch-3.json, .bee/cells/orchestrator-dispatch-4.json]
---

# orchestrator-dispatch — Delivery

## What shipped

- **orchestrator-dispatch-1** — Run state table, orchestration flag, domain + repository + engine accessors: `runs` table in SCHEMA, `orchestration_enabled` column on projects via MIGRATIONS (SCHEMA_VERSION → 2), `Run` struct, CRUD (insert/update-status/get/list), gating predicate (5 file(s) changed, commit 9f3d0a8)
- **orchestrator-dispatch-2** — Protocol engine (`orchestrate.rs`) over `&dyn Herdr`: fail-closed preflight (only Idle/Done/Unknown sendable; Working/Blocked/unavailable refuse), split HERDR_DONE_ marker mint (joined form never appears in the send text), Recent-source baseline capture, send, bounded await poll with fresh-marker completion rule and revision_of content-stability fallback for Unknown status; 9 FakeHerdr unit tests (2 file(s) changed, commit c83af4c)
- **orchestrator-dispatch-3** — Three MCP tools `waggledance_dispatch` / `waggledance_await` (60s server-side clamp) / `waggledance_runs` in mcp.rs, backed by a lazily-built owned tokio Runtime + SocketHerdr; D6 gating refusal names the remedy; preset-by-label only; spawn destination resolved from snapshot + Boundary containment, `agent_start` never called with cwd None (2 file(s) changed, commit 86c09bb)
- **orchestrator-dispatch-4** — Read-only Runs view `/p/:id/_runs` (task, worker pane, status, marker, timestamps; no mutating control) linked as a project tab, plus an Orchestration section on `/settings` with a per-project checkbox posting to `POST /api/projects/:id/orchestration` (2 file(s) changed, commit dc91cad)

The surface: an external LLM orchestrator holds only the three MCP tools — waggledance executes the mechanics, the orchestrator decides, workers code. Dispatch is refused unless the project's `orchestration_enabled` flag AND the global terminal family are both on (default off; the board stays read-only until a human opts a project in). Run state is durable in the registry DB, so a restarted orchestrator recovers its fleet by reading state rather than carrying a prompt roster.

## Verify

Each cell below was capped only against a recorded passing verify result — bee refuses a cap without one.

- **orchestrator-dispatch-1** — `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (1095 passed)
- **orchestrator-dispatch-2** — same suite (1111 passed; 9 new FakeHerdr protocol tests)
- **orchestrator-dispatch-3** — same suite (1124 passed; handler refusal-path tests)
- **orchestrator-dispatch-4** — same suite (1102 passed; route/render/toggle tests)

## Deviations

- **orchestrator-dispatch-1** — Added Project.orchestration_enabled field required test-fixture updates in indexer.rs and views.rs (outside the cell's declared files) to keep the workspace compiling; mechanical fallout, not a scope decision.

## Provenance

Proposed by `bee knowledge promote --work orchestrator-dispatch` from 4 capped cell trace(s) in `.bee/cells/` and the anchor `docs/history/orchestrator-dispatch/CONTEXT.md`, `docs/history/orchestrator-dispatch/plan.md`. Applied 2026-08-17 with What-shipped lines enriched from the capped worker reports and commit ids; pattern candidate (dispatch-1 fixture fallout) not promoted — the lesson ("a new required field on a shared domain struct breaks sibling fixtures") is generic language behavior, not a project pattern.
