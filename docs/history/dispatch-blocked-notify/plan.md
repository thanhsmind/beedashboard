---
artifact_contract: bee-plan/v1
mode: standard
# approved_gate2: <unset until approval>
---

# Plan: Dispatch Blocked Notify

Mode: `standard` — 2 risk flags: external-systems (the outbound notification
channel), multi-domain (orchestration + notification + persistence).
Why this is the least workflow that protects the work: the rules are already
locked in CONTEXT.md, so the only real risk left is wiring a new alert class
into a durable, at-least-once outbox without double-sending or spamming — two
slices with their own proofs cover exactly that and nothing more.

## Requirements (from CONTEXT.md)

- D1: a run reaching `Blocked` or `Timeout` notifies; `Done`/`Working` never do.
- D2: `failed` is out — no code writes it; nothing is invented for it here.
- D3: while a pane has an owning run, the run-aware alert is the only one sent
  for that event; the watcher's pane alert is suppressed for that pane.
- D4: the body names exactly project, pane, run id — no task text, no transcript.
- D5: at most one notification per run per status.
- D6: the existing opt-in notify switch governs; off means nothing is sent.

## Discovery

Inspected the three surfaces the feature joins. `await_run`
(`crates/waggledance/src/orchestrate.rs`) already persists **every**
terminal-for-this-call transition through `Engine::update_run_status` before
returning — so a single raise point exists and no new polling is needed.
`NotifyService::record` (`crates/waggledance/src/notify/mod.rs`) enqueues into
the outbox first and only marks delivered after a successful send, giving D5 a
natural home: dedupe belongs at enqueue, not at send. `NotifyStore`
(`crates/waggledance-core/src/notify_store.rs`) keys notifications by
`pane_id` and `kind` with no run identity and no uniqueness constraint, so D5
and D4 both need one schema change. Evidence: `rg -n "update_run_status|enqueue_notification|is_notifiable"`.

## Approach

Recommended path: keep one raise point and make dedupe a database property
rather than caller discipline. The run's status transition is the trigger (D1),
the outbox row carries the run id, and a uniqueness constraint on
(run id, status) makes a repeat enqueue a no-op (D5) even if a future caller
forgets. Suppression (D3) is a read of live run state at the watcher's enqueue
path, so the pane alert stays exactly as it is when no run owns the pane.
Nothing new is sent directly: everything still goes through the existing
outbox and drain, which keeps D6 true for free — a drained-by-nobody outbox
sends nothing when the switch is off.

Rejected alternatives:
- Raise the alert from `reconcile` by polling run rows — a second poller for
  state the await path already sees; more moving parts, and alerts that arrive
  behind the transition instead of with it.
- Dedupe in memory in the orchestrator — dies with the process, and D5's
  guarantee has to survive a restart.
- Have the run path call the notifier directly — bypasses the at-least-once
  outbox, so a send failure would silently lose the alert.

Risk map:

| Component | Risk | Reason | Proof needed |
|---|---|---|---|
| `notify_store` schema | MEDIUM | Existing databases already hold rows; the new column and index must apply to them, not just to fresh files | A test that opens an old-shape database, migrates, and still reads its rows |
| `orchestrate` raise point | LOW | The transition is already persisted in one place | `FakeHerdr` tests driving a pane to blocked and to timeout |
| D3 suppression | MEDIUM | Reads live run state from the watcher path; a wrong read either double-sends or silences a real pane alert | Tests both ways: with an owning run and without |
| Message body | LOW | Three identifiers, no free text | Assert the body carries project, pane, run — and no task text |

## Shape

| Phase | What Changes | Why Now | Demo | Unlocks |
|---|---|---|---|---|
| 1 | A run that goes `Blocked` or `Timeout` puts exactly one alert per status into the outbox, naming project, pane and run; the drain delivers it | This is the PBI's whole point, and it is end-to-end on its own — no stub, real delivery through the existing channel | Dispatch to a pane, let it block, watch one message arrive naming the run | The suppression rule, which only matters once run-aware alerts exist |
| 2 | While a pane has an owning run, the watcher's pane alert for that pane is suppressed | Without phase 1 there is nothing to suppress in favour of | Same dispatch: exactly one message, not two | — |

Slice queue: phase 1 (current) → phase 2 (headline only, prepared after
phase 1 caps).

## Test matrix

Triad, smallest demonstrating size:
- Happy path: a run transitioning to `Blocked` enqueues one alert whose body
  names project, pane, run id; the drain delivers it exactly once.
- Edge: the same run returning `Blocked` on repeated awaits enqueues nothing
  further; the same run then reaching `Timeout` enqueues a second alert (D5).
  A pre-existing database migrates without losing rows.
- Error paths: `Done` and `Working` enqueue nothing (D1); a send failure leaves
  the alert pending for the next drain (existing at-least-once behavior holds).

## Out of scope

- A `failed` run status and any alert for it (D2) — kept in CONTEXT.md under trigger `code-starts-writing-a-failed-run-status-a__cb4bc9f7`.
- Any resume, retry, or unblock action driven from the notification.
- The broadcast tool (`p-bf161077`, trigger `a-second-dispatched-run-alert-class-is-a__5a2fbc19`) and any change to the Telegram channel itself.
