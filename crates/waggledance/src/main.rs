//! waggledance — multi-project markdown viewer for AI agent workflows.

mod cli;
mod doctor;
mod herdr;
mod mcp;
mod notify;
mod orchestrate;
mod runtime;
mod server;
mod supervisor;
mod views;
mod watch;
mod watcher;

use clap::Parser;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

fn main() {
    // MCP speaks JSON-RPC on stdout; keep tracing on stderr and quiet by default.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "waggledance=info,warn".into()),
        )
        .init();

    let cli = cli::Cli::parse();
    if let Err(e) = cli::run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// D7's live-controllable pair: the herdr supervisor and the status
/// watcher/notifier, reconciled against a [`waggledance_core::config::TerminalConfig`]
/// on every switch write (`server::update_terminal_config`) and once at
/// startup (`server::serve`) — so flipping a switch takes effect
/// immediately, with **no restart**, and turning one off stops exactly the
/// task it started. This is the only place either module (agent-terminal-17)
/// is ever constructed: [`reconcile`](Self::reconcile) is the single
/// switch-on path, and it is a pure function of the `cfg` it is given — a
/// default `TerminalConfig` (both switches off) drives both branches below
/// to their `(false, false)` arm, which spawns nothing.
///
/// Turning a switch off is not just bookkeeping (agent-terminal-21's fix):
/// `reconcile_supervisor`/`reconcile_notify` cancel the running task
/// (`cancel` flag, checked by the supervisor immediately before its one
/// side-effecting spawn call — `supervisor::Supervisor::check_once`) *and*
/// keep its `JoinHandle` around so the next switch-on waits for it to
/// actually finish before starting a new one — closing the window where a
/// rapid off-then-on could otherwise leave two tasks briefly able to act at
/// once. `supervisor_running`/`notify_running` report this manager's own
/// bookkeeping (a slot holding a task); `supervisor_ticks`/`notify_ticks`
/// report a real, externally observable side effect of the task actually
/// still looping — the second pair is what proves "off" really stopped the
/// task rather than merely emptying a slot (see this cell's trace for the
/// red/green mutation that tells them apart).
#[derive(Default)]
pub struct TerminalBackground {
    supervisor: Mutex<Option<SupervisorTask>>,
    notify: Mutex<Option<JoinHandle<()>>>,
    /// The most recently stopped task's handle, kept only until the next
    /// switch-on has waited for it to finish.
    supervisor_stopping: Mutex<Option<JoinHandle<()>>>,
    notify_stopping: Mutex<Option<JoinHandle<()>>>,
    /// Incremented once per completed health check while the supervisor
    /// task is actually running — a real side effect, not bookkeeping.
    supervisor_ticks: Arc<AtomicU64>,
    /// Incremented once per completed poll cycle while the notify task is
    /// actually running.
    notify_ticks: Arc<AtomicU64>,
}

/// The supervisor's live task plus the flag that lets its owner cancel the
/// next spawn immediately, without waiting for `abort()` to land at the
/// task's next `.await` point.
struct SupervisorTask {
    handle: JoinHandle<()>,
    cancel: Arc<AtomicBool>,
}

impl TerminalBackground {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the supervisor task is live in this manager's own
    /// bookkeeping — queried by tests and available to a settings surface
    /// wanting to show "running" rather than only the stored switch value.
    pub fn supervisor_running(&self) -> bool {
        self.supervisor.lock().unwrap().is_some()
    }

    /// True while the notify (watcher + drain) task is live.
    pub fn notify_running(&self) -> bool {
        self.notify.lock().unwrap().is_some()
    }

    /// How many health checks the supervisor task has actually completed —
    /// a real side effect of the loop still running, unlike
    /// [`supervisor_running`](Self::supervisor_running)'s bookkeeping.
    /// Only this module's own tests read it today.
    #[cfg(test)]
    fn supervisor_ticks(&self) -> u64 {
        self.supervisor_ticks.load(Ordering::SeqCst)
    }

    /// How many poll cycles the notify task has actually completed. Only
    /// this module's own tests read it today.
    #[cfg(test)]
    fn notify_ticks(&self) -> u64 {
        self.notify_ticks.load(Ordering::SeqCst)
    }

    /// Start what `cfg` says should be running and isn't; stop (abort) what
    /// is running and shouldn't be. A switch already in the state `cfg`
    /// wants is left untouched — flipping the *other* switch never disturbs
    /// this one, and flipping the same switch on twice in a row is a no-op
    /// the second time.
    ///
    /// `telegram` is `Some((token, chat_id))` only when both halves of the
    /// notify destination/credential are configured (`server::telegram_credentials`)
    /// — `None` falls back to `notify::NullNotifier`, which only logs, so a
    /// configuration missing either half never attempts a delivery even
    /// with the switch on.
    pub fn reconcile(
        &self,
        cfg: &waggledance_core::config::TerminalConfig,
        herdr: Arc<dyn herdr::Herdr>,
        notify_store: Arc<waggledance_core::notify_store::NotifyStore>,
        telegram: Option<(String, String)>,
    ) {
        self.reconcile_supervisor(cfg.supervisor_enabled, herdr.clone());
        self.reconcile_notify(cfg.notify_enabled, herdr, notify_store, telegram);
    }

    fn reconcile_supervisor(&self, enabled: bool, control: Arc<dyn herdr::Herdr>) {
        self.reconcile_supervisor_with_intervals(
            enabled,
            control,
            Duration::from_secs(5),
            Duration::from_secs(3),
        );
    }

    /// `interval`/`backoff` are parameterized only so a test can drive the
    /// loop fast enough to observe real ticks without taking multiple
    /// seconds; every production call goes through
    /// [`reconcile_supervisor`](Self::reconcile_supervisor)'s fixed values.
    fn reconcile_supervisor_with_intervals(
        &self,
        enabled: bool,
        control: Arc<dyn herdr::Herdr>,
        interval: Duration,
        backoff: Duration,
    ) {
        let mut slot = self.supervisor.lock().unwrap();
        match (enabled, slot.take()) {
            (true, Some(existing)) => *slot = Some(existing), // already running
            (true, None) => {
                // Wait for whatever the previous "off" was stopping before
                // this task's loop does anything observable (pings,
                // spawns) — the sync `reconcile` call itself never blocks;
                // the wait happens inside the new task.
                let previous = self.supervisor_stopping.lock().unwrap().take();
                let cancelled = Arc::new(AtomicBool::new(false));
                let sup = supervisor::Supervisor::with_cancel_flag(
                    control,
                    Arc::new(supervisor::SpawnHerdr {
                        binary: supervisor::herdr_binary_from_env(),
                        // waggledance has no multi-session concept of its own
                        // today (only `default_socket_path()` is ever
                        // resolved) — "default" is the session
                        // `resolve_socket_path` treats identically to the
                        // legacy single-socket path this whole feature
                        // already talks to.
                        session: "default".to_string(),
                    }),
                    interval,
                    backoff,
                    cancelled.clone(),
                );
                let ticks = self.supervisor_ticks.clone();
                let handle = tokio::spawn(async move {
                    if let Some(prev) = previous {
                        let _ = prev.await;
                    }
                    sup.run(
                        ticks,
                        |health| {
                            tracing::info!(?health, "herdr health transition");
                        },
                        |step, wait| {
                            tracing::warn!(
                                step,
                                ?wait,
                                "herdr still unavailable; backing off before the next restart"
                            );
                        },
                    )
                    .await;
                });
                *slot = Some(SupervisorTask {
                    handle,
                    cancel: cancelled,
                });
            }
            (false, Some(existing)) => {
                // Cancel immediately — checked by `Supervisor::check_once`
                // right before its next spawn — then abort the task, but
                // keep its handle so the next switch-on can wait for it to
                // actually be gone rather than racing it.
                existing.cancel.store(true, Ordering::SeqCst);
                existing.handle.abort();
                *self.supervisor_stopping.lock().unwrap() = Some(existing.handle);
            }
            (false, None) => {}
        }
    }

    fn reconcile_notify(
        &self,
        enabled: bool,
        control: Arc<dyn herdr::Herdr>,
        store: Arc<waggledance_core::notify_store::NotifyStore>,
        telegram: Option<(String, String)>,
    ) {
        self.reconcile_notify_with_interval(
            enabled,
            control,
            store,
            telegram,
            Duration::from_millis(2000),
        );
    }

    /// `interval` is parameterized for the same reason as
    /// [`reconcile_supervisor_with_intervals`](Self::reconcile_supervisor_with_intervals).
    fn reconcile_notify_with_interval(
        &self,
        enabled: bool,
        control: Arc<dyn herdr::Herdr>,
        store: Arc<waggledance_core::notify_store::NotifyStore>,
        telegram: Option<(String, String)>,
        interval: Duration,
    ) {
        let mut slot = self.notify.lock().unwrap();
        match (enabled, slot.take()) {
            (true, Some(existing)) => *slot = Some(existing), // already running
            (true, None) => {
                let previous = self.notify_stopping.lock().unwrap().take();
                let notifier: Arc<dyn notify::Notifier> = match telegram {
                    Some((token, chat_id)) => {
                        match notify::TelegramNotifier::new(Some(token), Some(chat_id)) {
                            Some(t) => Arc::new(t),
                            None => Arc::new(notify::NullNotifier),
                        }
                    }
                    None => Arc::new(notify::NullNotifier),
                };
                let service = Arc::new(notify::NotifyService::new(store, notifier));
                let poll_watcher = watcher::PollWatcher::new(control, interval);
                let ticks = self.notify_ticks.clone();
                *slot = Some(tokio::spawn(async move {
                    if let Some(prev) = previous {
                        let _ = prev.await;
                    }
                    poll_watcher
                        .run_async(ticks, move |change| {
                            let service = service.clone();
                            async move {
                                tracing::info!(
                                    pane = %change.pane_id,
                                    status = change.status.as_str(),
                                    "agent status change"
                                );
                                if service.record(&change).await {
                                    service.drain().await;
                                }
                            }
                        })
                        .await;
                }));
            }
            (false, Some(handle)) => {
                handle.abort();
                *self.notify_stopping.lock().unwrap() = Some(handle);
            }
            (false, None) => {}
        }
    }
}

impl Drop for TerminalBackground {
    /// Belt-and-suspenders: a live task's runtime is already torn down with
    /// the process (or, in tests, with the single-test `#[tokio::test]`
    /// runtime) — this just makes cancellation immediate rather than
    /// implicit, and keeps a `TerminalBackground` dropped mid-test from
    /// leaving a supervisor loop's next `sleep` tick pending.
    fn drop(&mut self) {
        if let Some(t) = self.supervisor.lock().unwrap().take() {
            t.cancel.store(true, Ordering::SeqCst);
            t.handle.abort();
        }
        if let Some(h) = self.notify.lock().unwrap().take() {
            h.abort();
        }
        if let Some(h) = self.supervisor_stopping.lock().unwrap().take() {
            h.abort();
        }
        if let Some(h) = self.notify_stopping.lock().unwrap().take() {
            h.abort();
        }
    }
}

#[cfg(test)]
mod terminal_background_tests {
    //! D7 boundary, proved behaviorally now that `TerminalBackground` makes
    //! the constructions this module used to forbid by source-scanning
    //! `main.rs` (see the type's own doc comment for why that test shape no
    //! longer applies). Every test here uses `FakeHerdr` (default: up) and
    //! an in-memory `NotifyStore`, so nothing here ever spawns a real
    //! process or reaches the network — a live-down `FakeHerdr` is never
    //! configured, so `check_once`'s restart branch (which really would
    //! spawn `herdr`) never fires.
    use super::*;
    use crate::herdr::fake::FakeHerdr;
    use waggledance_core::config::TerminalConfig;
    use waggledance_core::notify_store::NotifyStore;

    fn store() -> Arc<NotifyStore> {
        Arc::new(NotifyStore::open_in_memory().unwrap())
    }

    /// The single most important D7 proof: a default (never-configured)
    /// `TerminalConfig` — both switches off — reconciled against a live
    /// `TerminalBackground` starts neither task. Removing either `(true,
    /// None) => { … tokio::spawn …}` arm entirely would not turn this test
    /// red (this cfg never reaches that arm) — it is
    /// `switch_on_starts_the_task_switch_off_stops_it` below that catches
    /// that half; this test instead catches a bug where the `(false, _)`
    /// arms accidentally spawn regardless of `enabled`.
    #[tokio::test]
    async fn default_config_starts_nothing() {
        let bg = TerminalBackground::new();
        bg.reconcile(
            &TerminalConfig::default(),
            Arc::new(FakeHerdr::new()),
            store(),
            None,
        );
        assert!(!bg.supervisor_running());
        assert!(!bg.notify_running());
    }

    /// The supervisor switch: on starts the watchdog, off stops it — with no
    /// restart in between, both changes going through the same
    /// `TerminalBackground`. This is this manager's own bookkeeping
    /// (`slot.take()` empties the slot unconditionally as part of the
    /// match, before either arm's body runs) — it proves the switch is
    /// tracked correctly, but a cancellation that silently did nothing
    /// would leave this test green too (agent-terminal-21's finding, and
    /// the reason `docs/history/learnings/20260805-toothless-security-assertions.md`
    /// applies here). `supervisor_off_actually_stops_the_watchdog` below is
    /// the test with teeth: it observes a real side effect of the task
    /// having stopped, not just this bookkeeping.
    #[tokio::test]
    async fn supervisor_switch_on_starts_the_watchdog_off_stops_it() {
        let bg = TerminalBackground::new();
        let mut cfg = TerminalConfig {
            supervisor_enabled: true,
            ..Default::default()
        };

        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(
            bg.supervisor_running(),
            "switching on must start the watchdog"
        );
        assert!(!bg.notify_running(), "the notify switch is still off");

        cfg.supervisor_enabled = false;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(
            !bg.supervisor_running(),
            "switching off must stop the watchdog"
        );
    }

    /// The notify switch: on starts the watcher/drain task, off stops it —
    /// same bookkeeping-only shape (and the same caveat) as the supervisor
    /// test above; `notify_off_actually_stops_the_watcher` below is the
    /// test with teeth.
    #[tokio::test]
    async fn notify_switch_on_starts_the_watcher_off_stops_it() {
        let bg = TerminalBackground::new();
        let mut cfg = TerminalConfig {
            notify_enabled: true,
            ..Default::default()
        };

        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.notify_running(), "switching on must start the watcher");
        assert!(
            !bg.supervisor_running(),
            "the supervisor switch is still off"
        );

        cfg.notify_enabled = false;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(!bg.notify_running(), "switching off must stop the watcher");
    }

    /// Flipping one switch never disturbs the other — each `reconcile_*`
    /// call only ever touches its own slot.
    #[tokio::test]
    async fn switches_are_independent() {
        let bg = TerminalBackground::new();
        let mut cfg = TerminalConfig {
            supervisor_enabled: true,
            notify_enabled: true,
            ..Default::default()
        };
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.supervisor_running());
        assert!(bg.notify_running());

        cfg.notify_enabled = false;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(
            bg.supervisor_running(),
            "turning off notify must not touch the supervisor"
        );
        assert!(!bg.notify_running());
    }

    /// Reconciling twice with the same "on" config must not re-spawn (no
    /// observable effect here beyond not panicking/leaking — the `(true,
    /// Some(existing)) => *slot = Some(existing)` arm is what this exercises).
    #[tokio::test]
    async fn reconciling_an_already_running_switch_is_a_no_op() {
        let bg = TerminalBackground::new();
        let cfg = TerminalConfig {
            supervisor_enabled: true,
            ..Default::default()
        };
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.supervisor_running());
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.supervisor_running());
    }

    /// `main.rs` must still declare the modules this manager depends on —
    /// carried over from the previous guard so a future accidental removal
    /// of a `mod` line is still caught, even though the rest of that guard
    /// no longer applies (see the module doc comment).
    #[test]
    fn main_declares_the_background_modules() {
        let src = include_str!("main.rs");
        for m in ["mod notify;", "mod watcher;", "mod supervisor;"] {
            assert!(src.contains(m), "main.rs must declare `{m}`");
        }
    }

    /// Defect (1), the test with teeth: `supervisor_ticks` is a real side
    /// effect of the watchdog loop still running, not this manager's own
    /// bookkeeping. Manually verified red/green: replacing
    /// `existing.handle.abort()` in `reconcile_supervisor_with_intervals`'s
    /// `(false, Some(existing))` arm with a no-op turns this red (ticks
    /// keep advancing after "off") while leaving
    /// `supervisor_switch_on_starts_the_watchdog_off_stops_it` above green.
    #[tokio::test]
    async fn supervisor_off_actually_stops_the_watchdog() {
        let bg = TerminalBackground::new();
        let fake: Arc<dyn crate::herdr::Herdr> = Arc::new(FakeHerdr::new());
        let fast = Duration::from_millis(15);

        bg.reconcile_supervisor_with_intervals(true, fake.clone(), fast, fast);
        // Let the watchdog actually tick a few times before turning it off.
        tokio::time::sleep(Duration::from_millis(90)).await;
        let ticks_while_on = bg.supervisor_ticks();
        assert!(
            ticks_while_on >= 2,
            "the watchdog must actually run while switched on (ticks={ticks_while_on})"
        );

        bg.reconcile_supervisor_with_intervals(false, fake.clone(), fast, fast);
        let ticks_at_off = bg.supervisor_ticks();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let ticks_after_wait = bg.supervisor_ticks();
        assert_eq!(
            ticks_at_off, ticks_after_wait,
            "switching off must stop the watchdog from ticking again — \
             a no-op cancellation would let this keep advancing"
        );
    }

    /// Same proof, against the notify task's poll cycles.
    #[tokio::test]
    async fn notify_off_actually_stops_the_watcher() {
        let bg = TerminalBackground::new();
        let fake: Arc<dyn crate::herdr::Herdr> = Arc::new(FakeHerdr::new());
        let fast = Duration::from_millis(15);

        bg.reconcile_notify_with_interval(true, fake.clone(), store(), None, fast);
        tokio::time::sleep(Duration::from_millis(90)).await;
        let ticks_while_on = bg.notify_ticks();
        assert!(
            ticks_while_on >= 2,
            "the watcher must actually poll while switched on (ticks={ticks_while_on})"
        );

        bg.reconcile_notify_with_interval(false, fake.clone(), store(), None, fast);
        let ticks_at_off = bg.notify_ticks();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let ticks_after_wait = bg.notify_ticks();
        assert_eq!(
            ticks_at_off, ticks_after_wait,
            "switching off must stop the watcher from polling again"
        );
    }

    /// A `Herdr` whose `ping` takes a while and counts how many calls are
    /// ever in flight at once — every other method is unreachable from the
    /// supervisor path and panics if that assumption ever breaks. Lets
    /// `switching_off_then_on_never_leaves_two_supervisors_pinging_at_once`
    /// observe overlap directly rather than inferring it from timing.
    struct SlowPingHerdr {
        in_flight: Arc<std::sync::atomic::AtomicUsize>,
        max_in_flight: Arc<std::sync::atomic::AtomicUsize>,
        delay: Duration,
    }

    /// Decrements `in_flight` on drop rather than after `sleep` returns —
    /// an aborted task's `ping()` future is dropped mid-`sleep` (cancellation
    /// never runs code *after* the await it lands on), so decrementing only
    /// on normal completion would leave a cancelled ping's slot stuck
    /// "in flight" forever and falsely report overlap that never happened.
    struct InFlightGuard(Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for InFlightGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl crate::herdr::Herdr for SlowPingHerdr {
        async fn snapshot(&self) -> crate::herdr::Result<crate::herdr::Snapshot> {
            unimplemented!("not exercised by the supervisor")
        }
        async fn ping(&self) -> crate::herdr::Result<crate::herdr::ProtocolInfo> {
            let now = self
                .in_flight
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.max_in_flight
                .fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            let _guard = InFlightGuard(self.in_flight.clone());
            tokio::time::sleep(self.delay).await;
            Ok(crate::herdr::ProtocolInfo {
                protocol: crate::herdr::HERDR_PROTOCOL,
                server_version: "slow-fake".into(),
            })
        }
        async fn read_pane(
            &self,
            _pane_id: &str,
            _source: crate::herdr::ReadSource,
            _lines: usize,
        ) -> crate::herdr::Result<crate::herdr::ScreenRead> {
            unimplemented!("not exercised by the supervisor")
        }
        async fn send_input(
            &self,
            _pane_id: &str,
            _text: &str,
            _submit: bool,
        ) -> crate::herdr::Result<()> {
            unimplemented!("not exercised by the supervisor")
        }
        async fn send_text(&self, _pane_id: &str, _bytes: &str) -> crate::herdr::Result<()> {
            unimplemented!("not exercised by the supervisor")
        }
        async fn send_keys(&self, _pane_id: &str, _keys: &[String]) -> crate::herdr::Result<()> {
            unimplemented!("not exercised by the supervisor")
        }
        async fn tab_create(
            &self,
            _workspace_id: &str,
            _cwd: Option<&str>,
        ) -> crate::herdr::Result<crate::herdr::TabCreated> {
            unimplemented!("not exercised by the supervisor")
        }
        async fn agent_start(
            &self,
            _workspace_id: &str,
            _cwd: Option<&str>,
            _argv: &[String],
        ) -> crate::herdr::Result<crate::herdr::AgentStarted> {
            unimplemented!("not exercised by the supervisor")
        }
    }

    /// Defect (5): a rapid off-then-on must never leave two supervisor
    /// generations able to ping (and, on a down herdr, spawn) at the same
    /// time — `reconcile_supervisor_with_intervals`'s new generation must
    /// wait for the previous one to actually be gone before it starts
    /// pinging at all.
    #[tokio::test]
    async fn switching_off_then_on_never_leaves_two_supervisors_pinging_at_once() {
        let bg = TerminalBackground::new();
        let in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let max_in_flight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let herdr: Arc<dyn crate::herdr::Herdr> = Arc::new(SlowPingHerdr {
            in_flight: in_flight.clone(),
            max_in_flight: max_in_flight.clone(),
            delay: Duration::from_millis(40),
        });
        let fast = Duration::from_millis(5);

        bg.reconcile_supervisor_with_intervals(true, herdr.clone(), fast, fast);
        // Let a ping actually be in flight before the risky rapid sequence.
        tokio::time::sleep(Duration::from_millis(10)).await;
        bg.reconcile_supervisor_with_intervals(false, herdr.clone(), fast, fast);
        bg.reconcile_supervisor_with_intervals(true, herdr.clone(), fast, fast);
        // Give both generations time to attempt overlap if the guard were broken.
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert_eq!(
            max_in_flight.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "at most one supervisor generation may ping herdr at a time, even across an off-then-on"
        );
    }
}
