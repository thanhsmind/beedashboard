//! Protocol engine — the mechanical Herdr protocol (D1/D5): preflight a
//! target pane, mint a fresh split marker, capture a pre-send baseline,
//! send the task, and poll for completion, all as functions over `&dyn
//! Herdr` so the whole protocol is testable against `FakeHerdr` with no
//! live herdr and no MCP process (`waggledance mcp`, a later slice, calls
//! these functions; it owns no protocol logic of its own — see
//! `docs/history/orchestrator-dispatch/plan.md`'s phase table).
//!
//! Fail-closed throughout (D5): a send is refused before any pane write
//! unless the target is verifiably `Idle`/`Done`/`Unknown`; a snapshot
//! failure is treated as unverifiable, never as "probably fine". Completion
//! is proven only by a *fresh* split marker — present in a current read and
//! absent from the run's own pre-send baseline — never by a marker string
//! that merely happens to already be on screen (a prior run's own echo, or
//! an agent quoting the instruction back). `Unknown` status (no agent
//! status this app trusts) falls back to three consecutive unchanged
//! `ansi::revision_of` reads as a settled-content proxy for completion.
//!
//! Every item here is exercised by this module's own `#[cfg(test)]` suite
//! (against `FakeHerdr`); its production caller is `mcp.rs`'s
//! `waggledance_dispatch`/`waggledance_await` tools (phase 3, per
//! `docs/history/orchestrator-dispatch/plan.md`'s phase table).

use std::time::Duration;

use waggledance_core::ansi;
use waggledance_core::domain::Run;
use waggledance_core::indexer::now_rfc3339;
use waggledance_core::notify_store::NotifyStore;
use waggledance_core::paths_boundary::Boundary;
use waggledance_core::Engine;

use crate::herdr::{self, AgentStatus, Herdr, HerdrError, ReadSource};
use crate::notify;

/// Herdr's own hard cap on a `recent` read (mirrors `pane_scroller.rs`'s own
/// local copy of the same constant, `pane_scroller.rs:52`) — baseline/delta
/// reads use `Recent`, capped the same way.
const RECENT_LINES_CAP: usize = 1000;

/// Hard cap on `await_run`'s wait (D4) — a caller-requested longer timeout
/// is silently clamped, never honored, never an error.
pub const MAX_AWAIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Production poll cadence for `await_run`'s status/content loop — long
/// enough not to hammer herdr with `snapshot`+`read_pane` calls every tick,
/// short enough not to waste much of a 60s budget on coarse granularity.
const AWAIT_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Consecutive unchanged `ansi::revision_of` reads required before an
/// `Unknown`-status pane is treated as settled (D5 content-stability
/// fallback).
const STABILITY_READS: u32 = 3;

/// The split marker's fixed half — never sent to a pane joined with its
/// suffix (see `Marker`'s own doc for why).
const MARKER_PREFIX: &str = "HERDR_DONE_";

/// Combines every failure `await_run` can hit: a herdr transport/protocol
/// failure, or a run-state persistence failure through the cell-1
/// repository methods.
#[derive(Debug, thiserror::Error)]
pub enum OrchestrateError {
    #[error(transparent)]
    Herdr(#[from] HerdrError),
    #[error("run persistence failed: {0}")]
    Store(#[from] waggledance_core::Error),
}

/// Why `preflight` refused a send, before any pane write happened (D5
/// fail-closed).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DispatchRefusal {
    #[error("pane {0} is working -- refusing to interrupt an in-flight run")]
    Working(String),
    #[error("pane {0} is blocked -- refusing to send while it waits on a human")]
    Blocked(String),
    #[error("herdr snapshot unavailable -- pane {pane_id} status is unverifiable: {reason}")]
    Unverifiable { pane_id: String, reason: String },
    #[error("no such pane: {0}")]
    NoSuchPane(String),
    #[error("pane {pane_id} is not inside project {project_id}'s own root -- refusing to dispatch across project boundaries")]
    OutsideBoundary { pane_id: String, project_id: String },
}

/// Confirm `pane_id` names a pane whose own folder resolves **inside**
/// `boundary` (D6 per-project containment): the dispatch destination is only
/// ever a pane the calling project already owns, never one enumerated off
/// another project on the same host. Mirrors `server.rs`'s `project_panes`
/// containment rule exactly — a pane is in-project when its `cwd`, or failing
/// that its `foreground_cwd`, `validate_existing`s under the boundary — which
/// is the same check every sibling pane-scoped write route runs through
/// `project_and_verify_pane_in_boundary`. The pane-spawn branch of
/// [`run_dispatch`] never needs this: it *creates* the pane under a
/// boundary-validated workspace anchor, so containment is structural there;
/// the caller-supplied-`pane_id` branch is the one that must prove it.
///
/// `project_id` is carried only for the refusal message. A `pane_id` absent
/// from the snapshot's `panes[]`, or present but with no folder resolving
/// inside the boundary, both refuse — the second is the cross-project attack
/// this closes, the first is a stale/unknown pane, and neither is ever
/// treated as "probably fine".
pub fn verify_pane_in_boundary(
    snapshot: &herdr::Snapshot,
    boundary: &Boundary,
    pane_id: &str,
    project_id: &str,
) -> Result<(), DispatchRefusal> {
    let contained = snapshot
        .panes
        .iter()
        .filter(|pane| pane.pane_id == pane_id)
        .any(|pane| {
            pane.cwd
                .as_deref()
                .and_then(|raw| boundary.validate_existing(std::path::Path::new(raw)).ok())
                .or_else(|| {
                    pane.foreground_cwd
                        .as_deref()
                        .and_then(|raw| boundary.validate_existing(std::path::Path::new(raw)).ok())
                })
                .is_some()
        });
    if contained {
        Ok(())
    } else {
        Err(DispatchRefusal::OutsideBoundary {
            pane_id: pane_id.to_string(),
            project_id: project_id.to_string(),
        })
    }
}

/// Resolve `pane_id`'s current [`AgentStatus`] from a fresh snapshot and
/// gate the send on it (D5): only `Idle`/`Done`/`Unknown` are sendable.
/// `Working`/`Blocked` refuse outright, and a snapshot failure (transport
/// down, protocol mismatch, any `HerdrError`) is treated as unverifiable and
/// refuses fail-closed — never "probably fine".
pub async fn preflight(herdr: &dyn Herdr, pane_id: &str) -> Result<AgentStatus, DispatchRefusal> {
    let snapshot = herdr
        .snapshot()
        .await
        .map_err(|e| DispatchRefusal::Unverifiable {
            pane_id: pane_id.to_string(),
            reason: e.to_string(),
        })?;
    let agent = snapshot
        .agents
        .iter()
        .find(|a| a.pane_id == pane_id)
        .ok_or_else(|| DispatchRefusal::NoSuchPane(pane_id.to_string()))?;
    match agent.status {
        AgentStatus::Working => Err(DispatchRefusal::Working(pane_id.to_string())),
        AgentStatus::Blocked => Err(DispatchRefusal::Blocked(pane_id.to_string())),
        AgentStatus::Done | AgentStatus::Idle | AgentStatus::Unknown => Ok(agent.status),
    }
}

/// A freshly-minted split completion marker (D5). [`Marker::joined`] is the
/// exact string a completed run must print — the value `await_run` searches
/// for — and it is never sent to the pane as a contiguous substring:
/// [`Marker::instruction`] spells the two halves as two separate quoted
/// tokens the agent is told to concatenate itself. Sending the joined
/// string directly would echo it straight into the pane's own screen the
/// moment it's typed, and `await_run`'s "present in a current read" check
/// would then see it before the agent had done anything at all — a
/// same-poll false completion, not a marker the agent actually produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    suffix: String,
}

impl Marker {
    /// The exact string a completed run prints — what `await_run` searches
    /// a current read for, and what a run's baseline is checked against for
    /// staleness.
    pub fn joined(&self) -> String {
        format!("{MARKER_PREFIX}{}", self.suffix)
    }

    /// The instruction appended to the task text — spells the marker as two
    /// separate tokens (never joined) so the send itself never echoes the
    /// completion string onto the pane's own screen.
    pub fn instruction(&self) -> String {
        format!(
            "When (and only when) the task above is fully complete, print the string \"{MARKER_PREFIX}\" immediately followed by \"{}\" -- concatenate the two with no space, no punctuation, and no line break between them -- on a line by itself.",
            self.suffix
        )
    }
}

/// Mint a fresh, unique split marker for one run (D5): a random 64-bit
/// suffix, formatted as hex.
pub fn mint_marker() -> Marker {
    Marker {
        suffix: format!("{:016x}", rand::random::<u64>()),
    }
}

/// Capture `pane_id`'s pre-send transcript — D5's baseline. A `Recent` read
/// is the only legal source for a scrollback-spanning capture (`Visible`
/// only sees the current on-screen rows). A run's baseline is compared
/// against every later `Recent` read to prove marker *freshness* and to
/// compute the transcript delta.
pub async fn capture_baseline(herdr: &dyn Herdr, pane_id: &str) -> herdr::Result<String> {
    let read = herdr
        .read_pane(pane_id, ReadSource::Recent, RECENT_LINES_CAP)
        .await?;
    Ok(read.text)
}

/// Send `task` into `pane_id`, followed by `marker`'s instruction, as one
/// submitted reply — the only pane write this module performs, and only
/// ever after a caller has already run [`preflight`] and captured a
/// baseline. `submit: true`: herdr's own send≠submit split
/// (`Herdr::send_input`'s doc) means the Enter has to be requested
/// explicitly for the task to actually reach the agent, not just sit typed
/// in the composer.
pub async fn send_task(
    herdr: &dyn Herdr,
    pane_id: &str,
    task: &str,
    marker: &Marker,
) -> herdr::Result<()> {
    let text = format!("{task}\n\n{}", marker.instruction());
    herdr.send_input(pane_id, &text, true).await
}

/// The transcript delta versus baseline — everything a `Recent` read has
/// gained since the run's own pre-send capture. `current` is expected to
/// carry `baseline` as a prefix (scrollback only grows forward); when it
/// doesn't (the buffer rotated past 1000 lines, dropping the baseline's own
/// oldest rows), the whole current read is returned rather than guessing at
/// an offset — a superset delta is safe, a wrong one is not.
fn delta_from_baseline(baseline: &str, current: &str) -> String {
    match current.strip_prefix(baseline) {
        Some(rest) => rest.to_string(),
        None => current.to_string(),
    }
}

/// Resolve the herdr workspace/cwd destination for spawning a fresh agent
/// pane (D3's preset-spawn path): the first workspace in `snapshot` whose D2
/// anchor (`Snapshot::anchor_cwd_for_workspace`) validates against
/// `boundary`. Mirrors `server.rs`'s `project_creation_destination` exactly
/// (same containment rule, same "first workspace that validates" scan) —
/// that function is private and `AppState`-typed, so the MCP dispatch path
/// (which has neither) gets its own copy here rather than a shared one.
/// `None` when no workspace qualifies: the caller refuses with a named
/// "destination unresolved" error rather than ever falling back to another
/// directory, and in particular never reaches [`Herdr::agent_start`]'s own
/// documented `cwd: None` fallback (herdr's own process directory) — every
/// caller of this function passes the resolved `cwd` as `Some`, never
/// `None`.
pub fn resolve_spawn_destination(
    snapshot: &herdr::Snapshot,
    boundary: &Boundary,
) -> Option<(String, String)> {
    snapshot.workspaces.iter().find_map(|w| {
        let anchor = snapshot.anchor_cwd_for_workspace(&w.workspace_id)?;
        let resolved = boundary
            .validate_existing(std::path::Path::new(&anchor))
            .ok()?;
        Some((
            w.workspace_id.clone(),
            resolved.to_string_lossy().into_owned(),
        ))
    })
}

/// Clamp a caller-requested await timeout to [`MAX_AWAIT_TIMEOUT`] (D4) — a
/// longer request is silently capped, never honored, never an error.
fn clamp_timeout(timeout: Duration) -> Duration {
    timeout.min(MAX_AWAIT_TIMEOUT)
}

/// A run's terminal-for-this-call status, as `await_run` determines it.
/// [`RunStatus::as_str`] is exactly what gets written through
/// `Engine::update_run_status` (`Run::status`'s own doc names the same
/// vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// Still going: no fresh marker, not blocked, and (when status is
    /// `Unknown`) content hasn't settled yet. Also the timeout case for a
    /// pane with a trustworthy non-`Unknown` status — the wait's own
    /// deadline elapsed with the pane still reading as working, so the run
    /// stays open for a later `await_run` call.
    Working,
    /// A fresh marker was found, or (status `Unknown`) the screen settled
    /// for `STABILITY_READS` consecutive polls.
    Done,
    /// The pane's own `AgentStatus` reports `Blocked` — waiting on a human,
    /// never resolved by more polling.
    Blocked,
    /// The wait's own deadline elapsed while the pane's status stayed
    /// `Unknown` (or missing from the snapshot entirely) and never settled
    /// — distinct from `Working`'s timeout case: here there was never a
    /// trustworthy signal to fall back on at all, not even "known still
    /// working".
    Timeout,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Working => "working",
            RunStatus::Done => "done",
            RunStatus::Blocked => "blocked",
            RunStatus::Timeout => "timeout",
        }
    }
}

/// `await_run`'s result: the terminal-for-this-call status plus the
/// transcript delta versus the run's own baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitOutcome {
    pub status: RunStatus,
    pub delta: String,
}

/// Bounded poll for `run`'s completion (D4/D5): `timeout` is clamped to
/// [`MAX_AWAIT_TIMEOUT`] and the loop never waits past its own deadline
/// regardless of poll cadence. Status-preferred: a `Blocked` pane returns
/// immediately; a fresh marker (present in a current `Recent` read, absent
/// from `run.baseline` — the staleness check that keeps a marker already
/// sitting in the baseline from ever counting) returns `Done` regardless of
/// status; an `Unknown`-status pane with no fresh marker falls back to
/// content stability, `STABILITY_READS` consecutive unchanged
/// `ansi::revision_of` reads. Every terminal-for-this-call transition is
/// persisted through `Engine::update_run_status` (cell-1's repository
/// methods) before returning, so a restarted orchestrator recovers the
/// fleet by reading run state (D7) instead of needing this call to have
/// been the one that saw it.
/// `notify_store: None` runs the same poll/persist loop with no alert path
/// configured -- a caller with no notification store simply raises nothing
/// (dbn-2's own contract).
pub async fn await_run(
    herdr: &dyn Herdr,
    engine: &Engine,
    run: &Run,
    timeout: Duration,
    notify_store: Option<&NotifyStore>,
) -> Result<AwaitOutcome, OrchestrateError> {
    await_run_with_poll_interval(
        herdr,
        engine,
        run,
        timeout,
        AWAIT_POLL_INTERVAL,
        notify_store,
    )
    .await
}

/// `await_run`'s real loop, parameterized on poll cadence — production
/// always goes through `await_run` and gets `AWAIT_POLL_INTERVAL`; tests
/// substitute a millisecond-scale interval so the loop's real timeout/
/// stability/marker logic runs against `FakeHerdr` without spending real
/// wall-clock time up to the 60s cap (same test-seam shape as
/// `SocketHerdr::with_settle_durations_for_test`).
async fn await_run_with_poll_interval(
    herdr: &dyn Herdr,
    engine: &Engine,
    run: &Run,
    timeout: Duration,
    poll_interval: Duration,
    notify_store: Option<&NotifyStore>,
) -> Result<AwaitOutcome, OrchestrateError> {
    let deadline = tokio::time::Instant::now() + clamp_timeout(timeout);
    // A marker string minted for THIS run but already sitting in ITS OWN
    // baseline can only mean the joined string reached the pane some other
    // way (e.g. a leaked instruction, or an unrelated echo) -- it can never
    // be evidence of completion, so staleness is decided once, up front,
    // from the run's own fixed baseline/marker pair, never re-derived from
    // a later read.
    let marker_is_stale_from_start = run.baseline.contains(run.marker.as_str());
    let mut stable_reads: u32 = 0;
    let mut last_revision: Option<u64> = None;

    loop {
        let snapshot = herdr.snapshot().await?;
        let status = snapshot
            .agents
            .iter()
            .find(|a| a.pane_id == run.pane_id)
            .map(|a| a.status);
        let read = herdr
            .read_pane(&run.pane_id, ReadSource::Recent, RECENT_LINES_CAP)
            .await?;
        let delta = delta_from_baseline(&run.baseline, &read.text);

        if status == Some(AgentStatus::Blocked) {
            return finish(engine, run, RunStatus::Blocked, delta, notify_store).await;
        }

        if !marker_is_stale_from_start && read.text.contains(run.marker.as_str()) {
            return finish(engine, run, RunStatus::Done, delta, notify_store).await;
        }

        if status == Some(AgentStatus::Unknown) || status.is_none() {
            let revision = ansi::revision_of(&read.text);
            if last_revision == Some(revision) {
                stable_reads += 1;
            } else {
                last_revision = Some(revision);
                stable_reads = 1;
            }
            if stable_reads >= STABILITY_READS {
                return finish(engine, run, RunStatus::Done, delta, notify_store).await;
            }
        } else {
            stable_reads = 0;
            last_revision = None;
        }

        let now = tokio::time::Instant::now();
        if now >= deadline {
            let timed_out_status = if status.is_none() || status == Some(AgentStatus::Unknown) {
                RunStatus::Timeout
            } else {
                RunStatus::Working
            };
            return finish(engine, run, timed_out_status, delta, notify_store).await;
        }
        let remaining = deadline.saturating_duration_since(now);
        tokio::time::sleep(poll_interval.min(remaining)).await;
    }
}

/// Persist `run`'s terminal-for-this-call status transition (D7) and, when
/// the status is one a human must clear (D1) and a notification store is
/// configured, enqueue exactly one run-aware alert through dbn-1's
/// `enqueue_run_notification` -- the body names only project, pane and run
/// id (D4), and the store's own `(run_id, kind)` uniqueness constraint
/// makes a repeat enqueue for an already-notified status a no-op (D5).
/// Nothing is sent from here: the alert lands in the outbox and the
/// existing drain delivers it, so the opt-in switch (D6) keeps governing
/// delivery untouched. `notify_store: None` still persists the status --
/// it just raises nothing.
async fn finish(
    engine: &Engine,
    run: &Run,
    status: RunStatus,
    delta: String,
    notify_store: Option<&NotifyStore>,
) -> Result<AwaitOutcome, OrchestrateError> {
    engine.update_run_status(&run.id, status.as_str(), &now_rfc3339(), None, None)?;
    if let Some(store) = notify_store {
        if notify::is_run_notifiable(status) {
            let body = format!("{} {} {}", run.project_id, run.pane_id, run.id);
            if let Err(e) = store.enqueue_run_notification(
                &run.id,
                &run.project_id,
                &run.pane_id,
                status.as_str(),
                &body,
            ) {
                tracing::warn!("failed to enqueue run notification for {}: {e}", run.id);
            }
        }
    }
    Ok(AwaitOutcome { status, delta })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::fake::FakeHerdr;
    use waggledance_core::{Config, SqliteStore};

    fn test_engine() -> Engine {
        Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default())
    }

    fn build_run(id: &str, pane_id: &str, baseline: &str, marker: &str) -> Run {
        let now = now_rfc3339();
        Run {
            id: id.to_string(),
            project_id: "proj-1".to_string(),
            pane_id: pane_id.to_string(),
            preset_label: None,
            task: "do the thing".to_string(),
            baseline: baseline.to_string(),
            marker: marker.to_string(),
            status: "working".to_string(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn preflight_refuses_a_working_pane() {
        let herdr = FakeHerdr::new();
        // w1:p1 is seeded Working (see FakeHerdr::new's doc).
        let err = preflight(&herdr, "w1:p1").await.unwrap_err();
        assert_eq!(err, DispatchRefusal::Working("w1:p1".to_string()));
    }

    #[tokio::test]
    async fn preflight_refuses_a_blocked_pane() {
        let herdr = FakeHerdr::new();
        // w1:p2 is seeded Blocked.
        let err = preflight(&herdr, "w1:p2").await.unwrap_err();
        assert_eq!(err, DispatchRefusal::Blocked("w1:p2".to_string()));
    }

    #[tokio::test]
    async fn preflight_refuses_when_herdr_is_unavailable() {
        let herdr = FakeHerdr::new();
        herdr.set_available(false);
        let err = preflight(&herdr, "w1:p1").await.unwrap_err();
        match err {
            DispatchRefusal::Unverifiable { pane_id, .. } => assert_eq!(pane_id, "w1:p1"),
            other => panic!("expected Unverifiable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_run_completes_on_a_fresh_marker() {
        let herdr = FakeHerdr::new();
        let engine = test_engine();
        // w2:p4 is seeded Idle -- a legal send target.
        let pane = "w2:p4";
        let baseline = capture_baseline(&herdr, pane).await.unwrap();
        let marker = mint_marker();
        let run = build_run("run-fresh", pane, &baseline, &marker.joined());
        engine.insert_run(&run).unwrap();

        send_task(&herdr, pane, &run.task, &marker).await.unwrap();
        // The agent's own later output prints the joined marker -- the only
        // way the joined string reaches the pane in this test.
        herdr
            .send_input(pane, &marker.joined(), false)
            .await
            .unwrap();

        let store = NotifyStore::open_in_memory().unwrap();
        let outcome = await_run(&herdr, &engine, &run, Duration::from_secs(5), Some(&store))
            .await
            .unwrap();
        assert_eq!(outcome.status, RunStatus::Done);
        assert!(
            outcome.delta.contains(&marker.joined()),
            "delta must carry the marker that proved completion: {:?}",
            outcome.delta
        );

        let stored = engine.get_run(&run.id).unwrap().unwrap();
        assert_eq!(
            stored.status, "done",
            "the transition must persist through the cell-1 repository methods"
        );
        assert!(
            store.undelivered().unwrap().is_empty(),
            "Done never notifies (D1)"
        );
    }

    #[tokio::test]
    async fn await_run_ignores_a_marker_already_present_in_the_baseline() {
        let herdr = FakeHerdr::new();
        let engine = test_engine();
        let pane = "w2:p4"; // Idle -- never transitions to Blocked in this test.
        let marker = mint_marker();
        let joined = marker.joined();
        // The baseline itself already carries the joined marker -- a stale
        // sighting, never fresh -- and the pane's current content matches it
        // exactly (seeded to the same text), so the only way `await_run`
        // could ever call this Done is by skipping the staleness guard.
        let baseline = format!("earlier output\n{joined}\nmore earlier output");
        herdr.seed_scroll_pane(pane, &baseline, &baseline, None);
        let run = build_run("run-stale", pane, &baseline, &joined);
        engine.insert_run(&run).unwrap();

        let outcome = await_run_with_poll_interval(
            &herdr,
            &engine,
            &run,
            Duration::from_millis(20),
            Duration::from_millis(5),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.status,
            RunStatus::Working,
            "a marker already sitting in the baseline must never count as completion"
        );
    }

    #[tokio::test]
    async fn await_run_times_out_while_working() {
        let herdr = FakeHerdr::new();
        let engine = test_engine();
        // w1:p1 is seeded Working and never transitions in this test.
        let pane = "w1:p1";
        let baseline = capture_baseline(&herdr, pane).await.unwrap();
        let marker = mint_marker(); // never sent/printed -- no completion signal.
        let run = build_run("run-timeout", pane, &baseline, &marker.joined());
        engine.insert_run(&run).unwrap();

        let store = NotifyStore::open_in_memory().unwrap();
        let outcome = await_run_with_poll_interval(
            &herdr,
            &engine,
            &run,
            Duration::from_millis(15),
            Duration::from_millis(5),
            Some(&store),
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.status,
            RunStatus::Working,
            "a timeout on a known-Working pane must report Working, and the run stays open"
        );

        let stored = engine.get_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.status, "working");
        assert!(
            store.undelivered().unwrap().is_empty(),
            "Working never notifies (D1)"
        );
    }

    #[tokio::test]
    async fn await_run_returns_blocked_when_the_pane_blocks_mid_run() {
        let herdr = FakeHerdr::new();
        let engine = test_engine();
        let store = NotifyStore::open_in_memory().unwrap();
        let pane = "w2:p4"; // seeded Idle -- flips to Blocked mid-poll below.
        let baseline = capture_baseline(&herdr, pane).await.unwrap();
        let marker = mint_marker();
        let run = build_run("run-blocked", pane, &baseline, &marker.joined());
        engine.insert_run(&run).unwrap();

        let flipper = herdr.clone();
        let flip_pane = pane.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            flipper
                .set_status(&flip_pane, AgentStatus::Blocked)
                .await
                .unwrap();
        });

        let outcome = await_run_with_poll_interval(
            &herdr,
            &engine,
            &run,
            Duration::from_secs(2),
            Duration::from_millis(5),
            Some(&store),
        )
        .await
        .unwrap();
        assert_eq!(outcome.status, RunStatus::Blocked);

        let stored = engine.get_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.status, "blocked");

        let pending = store.undelivered().unwrap();
        assert_eq!(pending.len(), 1, "exactly one pending alert (D5)");
        assert_eq!(pending[0].kind, "blocked");
        assert_eq!(pending[0].run_id.as_deref(), Some(run.id.as_str()));
        assert!(pending[0].body.contains(&run.project_id));
        assert!(pending[0].body.contains(&run.pane_id));
        assert!(pending[0].body.contains(&run.id));
        assert!(
            !pending[0].body.contains(&run.task),
            "alert body must never carry the run's task text (D4): {:?}",
            pending[0].body
        );
    }

    #[tokio::test]
    async fn await_run_repeated_blocked_await_leaves_one_pending_alert() {
        let herdr = FakeHerdr::new();
        let engine = test_engine();
        let store = NotifyStore::open_in_memory().unwrap();
        let pane = "w1:p2"; // seeded Blocked from the start (see FakeHerdr::new's doc).
        let baseline = capture_baseline(&herdr, pane).await.unwrap();
        let marker = mint_marker();
        let run = build_run("run-blocked-twice", pane, &baseline, &marker.joined());
        engine.insert_run(&run).unwrap();

        for _ in 0..2 {
            let outcome = await_run_with_poll_interval(
                &herdr,
                &engine,
                &run,
                Duration::from_millis(20),
                Duration::from_millis(5),
                Some(&store),
            )
            .await
            .unwrap();
            assert_eq!(outcome.status, RunStatus::Blocked);
        }

        let pending = store.undelivered().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "D5: a run returning Blocked on two consecutive awaits must still \
             leave exactly one pending alert"
        );
    }

    #[tokio::test]
    async fn await_run_timeout_status_enqueues_exactly_one_alert() {
        let herdr = FakeHerdr::new();
        let engine = test_engine();
        let store = NotifyStore::open_in_memory().unwrap();
        let pane = "w2:p4";
        // Unknown is never a trustworthy signal to fall back on, so a
        // deadline reached before content stabilizes reports Timeout, not
        // Working.
        herdr.set_status(pane, AgentStatus::Unknown).await.unwrap();
        let baseline = capture_baseline(&herdr, pane).await.unwrap();
        let marker = mint_marker(); // never printed -- content stays put.
        let run = build_run("run-timeout-alert", pane, &baseline, &marker.joined());
        engine.insert_run(&run).unwrap();

        // poll_interval > timeout so the loop's own `remaining` cap makes
        // the very first sleep land exactly on the deadline -- the second
        // iteration's deadline check fires with stable_reads still under
        // STABILITY_READS, before content could ever settle into a Done.
        let outcome = await_run_with_poll_interval(
            &herdr,
            &engine,
            &run,
            Duration::from_millis(10),
            Duration::from_millis(50),
            Some(&store),
        )
        .await
        .unwrap();
        assert_eq!(outcome.status, RunStatus::Timeout);

        let stored = engine.get_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.status, "timeout");

        let pending = store.undelivered().unwrap();
        assert_eq!(pending.len(), 1, "exactly one pending timeout alert");
        assert_eq!(pending[0].kind, "timeout");
        assert_eq!(pending[0].run_id.as_deref(), Some(run.id.as_str()));
    }

    #[tokio::test]
    async fn await_run_falls_back_to_content_stability_on_unknown_status() {
        let herdr = FakeHerdr::new();
        let engine = test_engine();
        let pane = "w2:p4";
        herdr.set_status(pane, AgentStatus::Unknown).await.unwrap();
        let fixed_text = "a screen that never changes\n❯ ";
        herdr.seed_scroll_pane(pane, fixed_text, fixed_text, None);
        let baseline = capture_baseline(&herdr, pane).await.unwrap();
        let marker = mint_marker(); // never printed -- only stability proves completion.
        let run = build_run("run-unknown-stable", pane, &baseline, &marker.joined());
        engine.insert_run(&run).unwrap();

        let outcome = await_run_with_poll_interval(
            &herdr,
            &engine,
            &run,
            Duration::from_millis(200),
            Duration::from_millis(2),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome.status,
            RunStatus::Done,
            "an Unknown-status pane with unchanging content must settle via stability, not time out"
        );

        let stored = engine.get_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.status, "done");
    }

    #[test]
    fn timeout_over_the_hard_cap_is_clamped() {
        assert_eq!(
            clamp_timeout(Duration::from_secs(120)),
            MAX_AWAIT_TIMEOUT,
            "a request over 60s must be clamped, never honored"
        );
        assert_eq!(
            clamp_timeout(Duration::from_secs(30)),
            Duration::from_secs(30),
            "a request under the cap passes through unchanged"
        );
    }

    fn workspace_with_anchor(
        workspace_id: &str,
        cwd: &std::path::Path,
    ) -> (
        herdr::wire::Workspace,
        herdr::wire::PaneLayout,
        herdr::wire::Pane,
    ) {
        let tab_id = format!("{workspace_id}-tab");
        let pane_id = format!("{workspace_id}-pane");
        (
            herdr::wire::Workspace {
                workspace_id: workspace_id.to_string(),
                label: workspace_id.to_string(),
                agent_status: AgentStatus::Idle,
                active_tab_id: Some(tab_id.clone()),
            },
            herdr::wire::PaneLayout {
                workspace_id: workspace_id.to_string(),
                tab_id: tab_id.clone(),
                focused_pane_id: Some(pane_id.clone()),
            },
            herdr::wire::Pane {
                pane_id,
                workspace_id: workspace_id.to_string(),
                tab_id,
                cwd: Some(cwd.to_string_lossy().into_owned()),
                foreground_cwd: None,
            },
        )
    }

    #[test]
    fn resolve_spawn_destination_picks_the_first_workspace_whose_anchor_is_contained() {
        let pid = std::process::id();
        let root =
            std::env::temp_dir().join(format!("waggledance-orchestrate-destination-in-{pid}"));
        let elsewhere =
            std::env::temp_dir().join(format!("waggledance-orchestrate-destination-out-{pid}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let boundary = Boundary::new(vec![root.clone()]).unwrap();

        let (w_out, l_out, p_out) = workspace_with_anchor("w-outside", &elsewhere);
        let (w_in, l_in, p_in) = workspace_with_anchor("w-inside", &root);
        let snapshot = herdr::Snapshot {
            workspaces: vec![w_out, w_in],
            layouts: vec![l_out, l_in],
            panes: vec![p_out, p_in],
            ..Default::default()
        };

        let (workspace_id, cwd) = resolve_spawn_destination(&snapshot, &boundary)
            .expect("the inside workspace's anchor must resolve");
        assert_eq!(workspace_id, "w-inside");
        assert_eq!(cwd, root.canonicalize().unwrap().to_string_lossy());

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&elsewhere).ok();
    }

    #[test]
    fn resolve_spawn_destination_is_none_when_no_workspace_anchor_resolves() {
        let pid = std::process::id();
        let root =
            std::env::temp_dir().join(format!("waggledance-orchestrate-destination-none-{pid}"));
        let elsewhere = std::env::temp_dir().join(format!(
            "waggledance-orchestrate-destination-none-out-{pid}"
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let boundary = Boundary::new(vec![root.clone()]).unwrap();

        let (w_out, l_out, p_out) = workspace_with_anchor("w-only-outside", &elsewhere);
        let snapshot = herdr::Snapshot {
            workspaces: vec![w_out],
            layouts: vec![l_out],
            panes: vec![p_out],
            ..Default::default()
        };

        assert!(resolve_spawn_destination(&snapshot, &boundary).is_none());

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&elsewhere).ok();
    }

    #[test]
    fn verify_pane_in_boundary_accepts_a_pane_whose_cwd_is_inside_the_project() {
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("waggledance-orchestrate-verify-in-{pid}"));
        std::fs::create_dir_all(&root).unwrap();
        let boundary = Boundary::new(vec![root.clone()]).unwrap();

        let (_w, _l, pane) = workspace_with_anchor("w-inside", &root);
        let pane_id = pane.pane_id.clone();
        let snapshot = herdr::Snapshot {
            panes: vec![pane],
            ..Default::default()
        };

        assert!(verify_pane_in_boundary(&snapshot, &boundary, &pane_id, "proj-1").is_ok());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn verify_pane_in_boundary_refuses_a_pane_in_another_project() {
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("waggledance-orchestrate-verify-out-{pid}"));
        let elsewhere =
            std::env::temp_dir().join(format!("waggledance-orchestrate-verify-out-other-{pid}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        // The project's boundary is `root`, but the target pane lives in a
        // sibling project's directory -- the exact cross-project dispatch this
        // check closes.
        let boundary = Boundary::new(vec![root.clone()]).unwrap();

        let (_w, _l, foreign) = workspace_with_anchor("w-other-project", &elsewhere);
        let foreign_id = foreign.pane_id.clone();
        let snapshot = herdr::Snapshot {
            panes: vec![foreign],
            ..Default::default()
        };

        let err = verify_pane_in_boundary(&snapshot, &boundary, &foreign_id, "proj-1").unwrap_err();
        assert_eq!(
            err,
            DispatchRefusal::OutsideBoundary {
                pane_id: foreign_id,
                project_id: "proj-1".to_string(),
            }
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&elsewhere).ok();
    }

    #[test]
    fn verify_pane_in_boundary_refuses_a_pane_absent_from_the_snapshot() {
        let pid = std::process::id();
        let root =
            std::env::temp_dir().join(format!("waggledance-orchestrate-verify-absent-{pid}"));
        std::fs::create_dir_all(&root).unwrap();
        let boundary = Boundary::new(vec![root.clone()]).unwrap();

        // A pane id that appears nowhere in panes[] (stale, or never existed)
        // refuses the same way a cross-project one does -- never "probably
        // fine".
        let snapshot = herdr::Snapshot::default();
        let err =
            verify_pane_in_boundary(&snapshot, &boundary, "ghost:pane", "proj-1").unwrap_err();
        assert_eq!(
            err,
            DispatchRefusal::OutsideBoundary {
                pane_id: "ghost:pane".to_string(),
                project_id: "proj-1".to_string(),
            }
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn verify_pane_in_boundary_falls_back_to_foreground_cwd() {
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("waggledance-orchestrate-verify-fg-{pid}"));
        let elsewhere =
            std::env::temp_dir().join(format!("waggledance-orchestrate-verify-fg-out-{pid}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        let boundary = Boundary::new(vec![root.clone()]).unwrap();

        // process cwd is outside the project, but the foreground child moved
        // into it -- `project_panes`' own second-chance rule, so containment
        // must honor it too.
        let pane = herdr::wire::Pane {
            pane_id: "w-fg-pane".to_string(),
            workspace_id: "w-fg".to_string(),
            tab_id: "w-fg-tab".to_string(),
            cwd: Some(elsewhere.to_string_lossy().into_owned()),
            foreground_cwd: Some(root.to_string_lossy().into_owned()),
        };
        let snapshot = herdr::Snapshot {
            panes: vec![pane],
            ..Default::default()
        };

        assert!(verify_pane_in_boundary(&snapshot, &boundary, "w-fg-pane", "proj-1").is_ok());

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&elsewhere).ok();
    }
}
