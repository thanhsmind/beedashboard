# Dispatch Blocked Notify — Context

**Feature slug:** dispatch-blocked-notify
**Date:** 2026-08-20
**Shaping session:** complete
**Scope:** Standard
**Domain types:** RUN

## Feature Boundary

When a dispatched run reaches a status only a human can clear, the human is
told through the existing notification channel, with enough identity to walk
to the right terminal — and told once. It ends at the message: nothing here
resumes, retries, or unblocks a run.

## Locked Decisions

These are fixed. Planning must implement them exactly — cited, never reinterpreted.
Changing one requires the user, a new D-ID or an explicit supersession note, never
a silent edit.

| ID | Decision | Rationale (only if it changes implementation) |
|----|----------|-----------------------------------------------|
| D1 | A dispatched run notifies the human when it reaches `Blocked` or `Timeout`. `Done` and `Working` never notify. (cb4bc9f7) | `Blocked` waits on a person and no amount of polling clears it; `Timeout` here is not the ordinary long task — that returns `Working` — but the pane never giving a trustworthy signal at all. |
| D2 | `failed` is not part of this feature: no code writes that status today, so it is neither notified nor invented. The moment a `failed` run status exists it joins D1's rule. (cb4bc9f7) | `RunStatus` carries only `Working`/`Done`/`Blocked`/`Timeout`; "failed" appears solely as vocabulary in a doc comment. |
| D3 | While a pane is running a dispatched run, the run-aware notification is the only one sent for that event; the watcher's pane-status notification is suppressed for that pane. (5a2fbc19) | The watcher already alerts on a blocked pane, so one blocked run would otherwise reach the human twice. The run-aware message carries strictly more identity, so it wins. |
| D4 | The message names exactly three things: the project, the pane, and the run id. No task text, no transcript excerpt. (ee3c8ad6) | The human opens the terminal for the rest. A transcript tail can carry tokens or keys; three identifiers remove that risk class instead of filtering for it. |
| D5 | At most one notification per run per status. A run returning `Blocked` on every await sends exactly one blocked message, and a second only when it moves to a different notifiable status. (6f7a6483) | The orchestrator loops `await_run` on a bounded timeout, so an un-deduped rule sends one message per poll — minutes of spam for one stuck worker. |
| D6 | These notifications ride the existing opt-in notify switch: switch off means nothing is sent and the event reaches the log only. (bcd6212d) | A new alert class must not become a way around an off switch. |

### Agent's Discretion

Where the notification is raised from (the await path, a reconcile pass, or the
run-status persistence point), how D3's suppression is expressed, and how D5's
per-run-per-status record is stored — all planning's call, provided the observable
rules above hold.

## Terms

| Term | Meaning in this feature |
|------|-------------------------|
| Run | One dispatch: one task sent to one pane, from baseline capture to a terminal status. Carries a run id. |
| Notifiable status | A run status a human must clear: `Blocked` or `Timeout` (D1). |
| Owning run | The run a pane is currently executing; what makes a pane's alert run-aware (D3). |

## Existing Code Context

From the quick scout only. Downstream agents read these before planning.

### Reusable Assets

- `crates/waggledance/src/notify/mod.rs` — the `Notifier` port, `NullNotifier`, `is_notifiable`, and `NotifyService::record`; at-least-once delivery already enqueues before sending.
- `crates/waggledance-core/src/notify_store.rs` — the durable store that already carries the de-duplication and delivery bookkeeping D5 needs.
- `crates/waggledance/src/notify/telegram.rs` — the one live channel; nothing in this feature is Telegram-specific.

### Established Patterns

- Hexagonal port + adapter for outbound alerts — a new alert class is a new call into `Notifier`, never a new channel.
- Opt-in wiring through `TerminalBackground` in `crates/waggledance/src/main.rs`: `reconcile` is the only place the notify service is driven.

### Integration Points

- `crates/waggledance/src/orchestrate.rs` — `RunStatus`, `AwaitOutcome`, and `await_run`, which persists every terminal-for-this-call transition through `Engine::update_run_status`.
- `crates/waggledance/src/watcher.rs` — `StatusChange` and the pane-status diff whose blocked alert D3 suppresses.

## Canonical References

- `docs/history/orchestrator-dispatch/CONTEXT.md` — v1's locked decisions, including the bounded-await loop (D4) and the fail-closed protocol rules (D5) this feature runs on top of.

## Outstanding Questions

Both planning questions are answered; nothing is outstanding.

- [x] Which single place raises the alert — the status-persistence point inside `await_run`'s `finish`, which already persists every terminal-for-this-call transition (dbn-2). The MCP process opens its own outbox against the server's own database (dbn-4).
- [x] Whether D3's suppression reads live run state — it reads live run state through a `RunOwnership` port answered from the engine, not a marker on the pane record (dbn-5).

## Deferred Ideas — triggers `code-starts-writing-a-failed-run-status-a__cb4bc9f7`, `a-second-dispatched-run-alert-class-is-a__5a2fbc19`

Out-of-scope ideas captured during shaping. Each carries the registered trigger that brings it back.

- A `failed` run status for infrastructure breakage (pane gone, send refused mid-run) — out of scope per D2; it would touch orchestrate, persistence, and the MCP schema. Trigger: `code-starts-writing-a-failed-run-status-a__cb4bc9f7`.
- Broadcast tool (port of `broadcast.sh`) — backlog row `p-bf161077`. Trigger: `a-second-dispatched-run-alert-class-is-a__5a2fbc19`.
