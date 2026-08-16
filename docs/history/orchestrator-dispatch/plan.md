---
artifact_contract: bee-plan/v1
mode: standard
---

# Plan: Orchestrator Dispatch

Mode: `standard` — 3 risk flags: data-model, external-systems, multi-domain
Why this is the least workflow that protects the work: a new machine-facing write surface over an external process plus a schema change needs a reviewed plan, but no hard-gate territory (no auth removal, no data loss path) pushes it to high-risk.

## Requirements (from CONTEXT.md)

- D1: Waggledance implements the mechanical protocol (preflight, baseline, split marker, wait) in Rust on the `Herdr` trait; the orchestrator stays an external LLM holding only the MCP tools.
- D2: V1 = exactly three MCP tools: dispatch, await, and a run-state read. No broadcast, no pane-close, no layout tools.
- D3: `dispatch` spawns via the preset-gated `agent_start` path or targets a running pane; presets only, never raw argv.
- D4: `dispatch` returns `run_id` immediately; `await(run_id, timeout ≤ 60s)` blocks at most the timeout, returns status (working/done/blocked/timeout) + transcript delta.
- D5: Fail-closed protocol semantics: refuse sends to working/blocked/unverifiable targets; completion proven only by a fresh split marker; content-stability fallback on `unknown`.
- D6: Per-project `orchestration.enabled`, effective only with `terminal.enabled`; refusal names the remedy; default off.
- D7: Durable run state in registry SQLite (project, pane, preset, task, baseline ref, marker, status, timestamps); successor recovers fleet by reading state.
- D8: Read-only Runs view per project projected from run state.

## Discovery

Inspected the herdr trait/wire, mcp.rs construction, config, repository migration pattern, ANSI helpers, and route wiring (gather digest, 2026-08-16):
- `AgentStatus` is already typed (`wire.rs:22-29`: Working/Blocked/Done/Idle/Unknown) and `Snapshot.agents` carries status + title — preflight is a snapshot lookup.
- `Herdr` trait is strictly request/response (`herdr/mod.rs:13`); `await` must be an internal poll loop (precedent: `send_input`'s settle path, `mod.rs:169-183`).
- `ReadSource` legal values are only `Visible`/`Recent` (`mod.rs:67-81`) — baseline/delta reads use `Recent` (cap 1000 lines mirrors herdr, `pane_scroller.rs:52`).
- `waggledance mcp` is a separate sync stdio process building its own `Engine` (`mcp.rs:22-72`), no herdr client and no tokio runtime today — dispatch tools need a small owned runtime + `SocketHerdr` in that process. Registry SQLite is already shared across the server and MCP processes (precedent: `ensure_project` from MCP while the daemon runs).
- No per-project config exists (`config.rs:12-19` all global) — the per-project flag lands as a column on the registry `projects` table via the `MIGRATIONS` list (`repository.rs:344-366`); a brand-new `runs` table goes in `SCHEMA` (`CREATE TABLE IF NOT EXISTS`, `repository.rs:440-472`).
- MCP stdio has no auth (matches terminal-open-access D1 posture) — the gates are `terminal.enabled` + the new per-project flag, not a token.
- `ansi::revision_of` (`ansi.rs:268`) gives change-detection hashing for the content-stability fallback; `ansi::to_html` handles safe rendering for the Runs view if deltas are shown.

## Approach

Recommended (cites D1-D8): a new `orchestrate` module in `crates/waggledance` owning the protocol as pure-ish functions over `&dyn Herdr` + a `RunStore` (testable with `FakeHerdr`); registry grows a `runs` table and an `orchestration_enabled` column; `mcp.rs` gains the three tools backed by an owned tokio runtime + `SocketHerdr`; `server.rs`/`views.rs` gain the read-only Runs page and the per-project settings toggle.

Rejected alternatives:
- MCP tools proxying through the HTTP server's API — adds an HTTP hop and a second contract for no isolation gain; both processes already share the registry DB.
- Per-project flag in `config.toml` as a project-id map — config has no per-project precedent; the registry row is the existing per-project home.
- Porting the skill's grid/layout phases — layout is a herdr UI concern; the dashboard is the fleet view (D2 scope).

Risk map: protocol correctness (marker freshness, fail-closed preflight) MEDIUM — proven by FakeHerdr unit tests per skill semantics; cross-process DB writes LOW — SQLite WAL/busy precedent exists; MCP runtime addition LOW — owned `Runtime::block_on` per call; MCP loop serialization MEDIUM-accepted — the stdio loop is serial (`mcp.rs:26-70`), so a blocked `await` delays every other response, including `ping`. Accepted by design: the stdio client is the orchestrator itself, which calls `await` only when it has nothing else to ask; the ≤60s clamp bounds the worst-case delay, and the clamp is enforced server-side (over-cap requests truncated to 60s).

Named deviation from CONTEXT Integration Points (scout-level, not a locked D-ID): the per-project flag lives as a registry `projects` column, not in `config.rs` — config has no per-project precedent (`config.rs:12-19`), the registry row is the existing per-project home. Recorded in the decisions log.

Transport-coverage note: `FakeHerdr` fuses text+submit into one write (`socket.rs:1396-1398`), so the multi-line paste and send≠submit split are pinned only by the existing mock-socket-server tests (`socket.rs:1399-1412`) — the plan relies on that existing coverage; `orchestrate` tests prove protocol logic above the transport.

## Shape

Phase plan (milestone-shaped):

| Phase | What Changes | Why Now | Demo | Unlocks |
|---|---|---|---|---|
| 1. Run state + flag | `runs` table in SCHEMA, `orchestration_enabled` column via `MIGRATIONS` (with the `SCHEMA_VERSION` bump the module pins, `repository.rs:357`), `Run` domain struct, repository CRUD, engine accessors + gating predicate | Everything else reads/writes this | `cargo test` on store round-trip + gating refusal | 2, 4 |
| 2. Protocol engine | `orchestrate` module: preflight (snapshot, fail-closed per D5), baseline capture (Recent read), split-marker mint, send (task + marker instruction via `send_input`), bounded await poll (status-preferred, fresh-marker gating, `revision_of` stability fallback), delta extraction | Core of D1/D4/D5, testable against `FakeHerdr` without any UI | FakeHerdr tests: refuse-working, refuse-blocked, marker-fresh completion, stale-marker ignored, timeout, unknown-status fallback | 3 |
| 3. MCP tools | `waggledance_dispatch` (project, preset label or pane_id, task), `waggledance_await` (run_id, timeout — clamped to 60s server-side), `waggledance_runs` (project?) in mcp.rs; owned tokio runtime + `SocketHerdr`; destination resolution for spawn (workspace + cwd from snapshot + `Boundary` containment — `server.rs`'s `project_creation_destination` is private/AppState-typed, so `orchestrate` gets its own resolution over `Snapshot` + `Boundary`, and `agent_start` is always called with an explicit resolved `cwd`, never `None`); D6 refusal wording; errors for unknown preset and "destination unresolved" | The orchestrator-facing contract | tool `tools/list` + dispatch refusal on non-enabled project (unit-level handler tests) | an LLM orchestrator can drive a fleet |
| 4. Runs view + toggle | `/p/:id/_runs` route + views render (read-only, D8 — the view never mutates); per-project orchestration toggles as a section on the existing global `/settings` page (checkbox list of registered projects, POST endpoint per project id) | Human visibility; the opt-in switch D6 needs a UI | page renders run rows; toggle flips the column | v2 items (broadcast, notify) |

Current slice: all four phases are one slice — the feature is one walking skeleton (dispatch → run row → await → visible in Runs view); phases are dependency-ordered cells, not separate slices.

## Test matrix

Triad, smallest demonstrating size:
- Happy: dispatch spawns (FakeHerdr) → run row persisted with baseline+marker → await sees fresh marker → status done, delta returned; Runs page lists the row.
- Edge: dispatch to an already-running idle pane (no spawn); await times out while working (returns working, run stays open); await returns `blocked` when the pane goes blocked mid-run; marker string present in baseline (stale) is not completion; `unknown` status settles via three stable `revision_of` cycles; concurrent second dispatch to the same pane refused (working); await timeout > 60s is clamped to 60s.
- Error: dispatch refused — project not orchestration-enabled (error names remedy), terminal family off, unknown preset label, unresolvable pane or destination unresolved, blocked pane, herdr unavailable (snapshot failure = unverifiable, fail-closed); await on unknown run_id; await surfaces herdr-unavailable as an error, never a completion.
Writers judge existing coverage first: repository tests follow existing store test patterns; ANSI/scroller behavior already pinned — do not re-test.

## Out of scope

- Broadcast tool (PBI p-bf161077), blocked-run notification (PBI p-190c7bdc).
- Any autonomous control loop inside waggledance (D1).
- Layout/grid management, pane close/teardown tools.
- Auth for the MCP stdio path (matches existing open-access posture; revisit only if the posture changes).
