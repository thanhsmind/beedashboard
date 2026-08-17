# Orchestrator Dispatch — Context

**Feature slug:** orchestrator-dispatch
**Date:** 2026-08-16
**Shaping session:** complete
**Scope:** Standard
**Domain types:** CALL | RUN | SEE

## Feature Boundary

Waggledance gains a machine-facing dispatch surface: three new MCP tools that
let an LLM orchestrator spawn preset agents, hand them tasks through a
verified send/wait protocol (ported from the herdr-agent-comms skill), and
read durable run state — plus a read-only Runs view on the dashboard. The
feature ends before any policy layer: no review loops, no broadcast, no
autonomous control loop inside waggledance.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never
reinterpreted. Changing one requires the user, a new D-ID or an explicit
supersession note, never a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | Hybrid architecture: waggledance implements the mechanical protocol (preflight, baseline capture, split completion marker, wait semantics) in Rust on the existing `Herdr` trait; the orchestrator stays an external LLM agent holding only the new MCP tools plus read access. Waggledance never decides what to dispatch — it only executes dispatches safely. (decisions log 34791df7) | "Never codes" is enforced by tool surface, not prose; deterministic Rust replaces the fragile LLM-run bash protocol. |
| D2 | V1 tool set is exactly three MCP tools: `dispatch`, `await`, and a fleet/run-state read. No broadcast, no pane-close, no layout tools in v1. (34791df7, backlog p-bf161077) | — |
| D3 | `dispatch(project, preset, task)` both spawns a new agent via the existing preset-gated `agent_start` path and sends the task through the baseline/marker protocol; targeting an already-running pane is the same tool with a pane target instead of a preset. Raw argv/env/cwd from the caller is never accepted — presets only. (0ce97bb1) | Reuses the settled agent-terminal preset security posture. |
| D4 | `dispatch` returns a `run_id` immediately. `await(run_id, timeout ≤ 60s)` blocks at most the timeout, then returns the current status — `working`, `done`, `blocked`, or `timeout` — plus the transcript delta since the run's baseline. The orchestrator loops on it; durable run state guarantees nothing is lost between calls. (6d291898) | Worker tasks can run hours; unbounded MCP calls hit client timeouts. |
| D5 | Protocol port keeps the skill's fail-closed semantics: a send is refused when the target is `working`, `blocked`, or unverifiable; completion is proven only by a fresh split `HERDR_DONE_`-style marker (present in the current read, absent from the pre-send baseline); status falls back to content-stability polling when the pane reports `unknown`. (34791df7) | These are the load-bearing safety rules of the source skill; they become code, not prose. |
| D6 | Opt-in is a per-project `orchestration.enabled` flag in settings, effective only when the terminal family (`terminal.enabled`) is already on. Dispatch against a non-enabled project is refused with an error that names how to enable it. The default for every project is off — the board stays read-only until a human opts a project in. (0dd28b02) | Separates human view/reply rights from machine auto-dispatch rights; preserves the README invariant by default. |
| D7 | Run state is durable in the registry SQLite: per run at minimum the project, target pane, preset, task text, baseline reference, marker, status, and timestamps. A restarted orchestrator (or a successor session) recovers the fleet by reading run state — no prompt-carried roster. (34791df7, 6d291898) | This is what shrinks the source skill's HANDOFF to a state read. |
| D8 | V1 ships a read-only Runs view on the dashboard per project — task, worker pane, status, marker, timestamps — projected from run state. The view never mutates anything. (bdf7aefe) | — |

### Agent's Discretion

Exact MCP tool names, run-state table schema beyond D7's minimum fields,
marker suffix format, transcript-delta capping, and where the Runs view sits
in the existing dashboard layout — all planning-level, constrained by the
D-IDs above and the settled agent-terminal patterns.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| run | One dispatch: one task sent to one pane, from baseline capture to a terminal status (done/blocked/failed/timeout). |
| dispatch | The act of (optionally spawning and) sending a task to a worker pane through the verified protocol. Never a raw send. |
| marker | The split completion token typed into the prompt in two halves; only its joined form in fresh output proves completion. |
| baseline | The pre-send transcript capture a run's delta and marker-freshness are measured against. |
| opt-in | `orchestration.enabled` per project, requiring `terminal.enabled`; absent or off means dispatch is refused. |

## Specific Ideas And References

- Source skill: `/home/thanhsmind/projects/AI/luongnv89-skill/skills/herdr-agent-comms/` — SKILL.md phases, `scripts/preflight_send.py` (exit 0/2/3/4 sendable/working/blocked/unverifiable), `scripts/wait_for_idle.py` (exit 0/1/2/3, status-enum primary + content-stability fallback, fresh-marker gating), `references/delivery-and-waiting.md` (baseline→marker→preflight→send→verify sequence).
- Policy-layer example the surface must be able to host later: `/home/thanhsmind/projects/AI/luongnv89-skill/skills/issue-work-loop/SKILL.md` (roles, GitHub-truth verification, USER-MERGE).

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance/src/herdr/mod.rs:150-231` — `Herdr` trait: `snapshot/ping/read_pane/send_input/send_keys/tab_create/agent_start`; `FakeHerdr` test double exists.
- `crates/waggledance/src/server.rs:3030-3077` — `terminal_create_agent`: preset-gated agent start with server-side workspace/cwd resolution (D3 reuses this path).
- `crates/waggledance-core/src/repository.rs` — registry SQLite; run-state table lands beside `projects` (D7).
- `crates/waggledance/src/mcp.rs:74-159` — MCP tool table + dispatch; new tools register here.

### Established Patterns

- Terminal route family gated by `terminal.enabled` + terminal_auth (agent-terminal delivery) — the orchestration flag layers on top (D6), it does not replace this.
- Agent presets (`config::AgentPreset`) as the only way callers name an agent command — carried into D3.
- Send-then-submit split at the socket layer (agent-terminal-11) — the protocol port must respect it.

### Integration Points

- `crates/waggledance/src/mcp.rs` — three new tools.
- `crates/waggledance-core/src/config.rs` — per-project `orchestration.enabled`.
- Dashboard views (`views.rs`, `assets/app.js`) — Runs view (D8).

## Canonical References

- `README.md:128-131` — the board-never-writes invariant D6 preserves by default.
- decisions log: 34791df7 (architecture), 0ce97bb1 (dispatch scope), 6d291898 (await), 0dd28b02 (opt-in), bdf7aefe (Runs view).

## Outstanding Questions

### Deferred To Planning

- [ ] Whether `await` transcript deltas are capped by lines or bytes, and how ANSI is stripped — answered by reading `pane_scroller.rs`/`ansi.rs` capabilities.
- [ ] Whether run state rows expire or archive — answered by looking at registry growth patterns; default to keep-forever if cheap.
- [ ] How the MCP stdio path authenticates relative to terminal_auth's HTTP token — answered by reading how existing MCP tools trust the local caller.

## Deferred Ideas

- Broadcast tool (port of broadcast.sh) — v2, PBI p-bf161077.
- Notify human on blocked run via Telegram channel — v2, PBI p-190c7bdc.
- Any autonomous control loop inside waggledance — explicitly out (D1).

## Handoff Note

CONTEXT.md is the source of truth. Decision IDs are stable. Planning reads locked
decisions, code context, canonical references, and deferred-to-planning questions.
Planning's Gate 2 shape stage and reviewing use locked decisions for coverage and UAT.
