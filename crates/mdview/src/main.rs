//! mdview — multi-project markdown viewer for AI agent workflows.

mod cli;
mod doctor;
mod herdr;
mod mcp;
mod notify;
mod runtime;
mod server;
mod supervisor;
mod terminal_auth;
mod views;
mod watch;
mod watcher;

use clap::Parser;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

fn main() {
    // MCP speaks JSON-RPC on stdout; keep tracing on stderr and quiet by default.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mdview=info,warn".into()),
        )
        .init();

    let cli = cli::Cli::parse();
    if let Err(e) = cli::run(cli) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// D7's live-controllable pair: the herdr supervisor and the status
/// watcher/notifier, reconciled against a [`mdview_core::config::TerminalConfig`]
/// on every switch write (`server::update_terminal_config`) and once at
/// startup (`server::serve`) — so flipping a switch takes effect
/// immediately, with **no restart**, and turning one off stops exactly the
/// task it started. This is the only place either module (agent-terminal-17,
/// inert until this cell) is ever constructed: [`reconcile`](Self::reconcile)
/// is the single switch-on path, and it is a pure function of the `cfg` it
/// is given — a default `TerminalConfig` (both switches off) drives both
/// branches below to their `(false, false)` arm, which spawns nothing.
///
/// This replaces agent-terminal-17's `inert_until_switched_on` module, which
/// proved D7 by scanning `main.rs`'s own source text for the constructor
/// calls this cell now legitimately makes — a source scan can no longer
/// prove anything once those calls are real production code. The tests
/// below prove the same guarantee behaviorally instead: a default config
/// leaves both task slots empty (nothing spawned), and each switch
/// independently starts and then stops its own task, verified by removing
/// each half of `reconcile`'s branching in turn and confirming the
/// corresponding assertion goes red (see this cell's trace).
#[derive(Default)]
pub struct TerminalBackground {
    supervisor: Mutex<Option<JoinHandle<()>>>,
    notify: Mutex<Option<JoinHandle<()>>>,
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
        cfg: &mdview_core::config::TerminalConfig,
        herdr: Arc<dyn herdr::Herdr>,
        notify_store: Arc<mdview_core::notify_store::NotifyStore>,
        telegram: Option<(String, String)>,
    ) {
        self.reconcile_supervisor(cfg.supervisor_enabled, herdr.clone());
        self.reconcile_notify(cfg.notify_enabled, herdr, notify_store, telegram);
    }

    fn reconcile_supervisor(&self, enabled: bool, control: Arc<dyn herdr::Herdr>) {
        let mut slot = self.supervisor.lock().unwrap();
        match (enabled, slot.take()) {
            (true, Some(existing)) => *slot = Some(existing), // already running
            (true, None) => {
                let sup = supervisor::Supervisor::new(
                    control,
                    Arc::new(supervisor::SpawnHerdr {
                        binary: supervisor::herdr_binary_from_env(),
                        // mdview has no multi-session concept of its own
                        // today (only `default_socket_path()` is ever
                        // resolved) — "default" is the session
                        // `resolve_socket_path` treats identically to the
                        // legacy single-socket path this whole feature
                        // already talks to.
                        session: "default".to_string(),
                    }),
                    std::time::Duration::from_secs(5),
                    std::time::Duration::from_secs(3),
                );
                *slot = Some(tokio::spawn(sup.run(|health| {
                    tracing::info!(?health, "herdr health transition");
                })));
            }
            (false, Some(handle)) => handle.abort(), // stop; slot already emptied by take()
            (false, None) => {}
        }
    }

    fn reconcile_notify(
        &self,
        enabled: bool,
        control: Arc<dyn herdr::Herdr>,
        store: Arc<mdview_core::notify_store::NotifyStore>,
        telegram: Option<(String, String)>,
    ) {
        let mut slot = self.notify.lock().unwrap();
        match (enabled, slot.take()) {
            (true, Some(existing)) => *slot = Some(existing), // already running
            (true, None) => {
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
                let poll_watcher =
                    watcher::PollWatcher::new(control, std::time::Duration::from_millis(2000));
                *slot = Some(tokio::spawn(async move {
                    poll_watcher
                        .run_async(move |change| {
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
            (false, Some(handle)) => handle.abort(),
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
        if let Some(h) = self.supervisor.lock().unwrap().take() {
            h.abort();
        }
        if let Some(h) = self.notify.lock().unwrap().take() {
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
    use mdview_core::config::TerminalConfig;
    use mdview_core::notify_store::NotifyStore;

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
    /// `TerminalBackground`. Verified red/green by hand: commenting out the
    /// `(true, None) => { … }` arm's spawn fails the first assertion;
    /// commenting out the `(false, Some(handle)) => handle.abort()` arm
    /// (replacing it with a no-op that leaves `slot` as `Some`) fails the
    /// second.
    #[tokio::test]
    async fn supervisor_switch_on_starts_the_watchdog_off_stops_it() {
        let bg = TerminalBackground::new();
        let mut cfg = TerminalConfig::default();

        cfg.supervisor_enabled = true;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.supervisor_running(), "switching on must start the watchdog");
        assert!(!bg.notify_running(), "the notify switch is still off");

        cfg.supervisor_enabled = false;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(!bg.supervisor_running(), "switching off must stop the watchdog");
    }

    /// The notify switch: on starts the watcher/drain task, off stops it —
    /// same shape and same manual red/green verification as the supervisor
    /// test above, against the notify arms instead.
    #[tokio::test]
    async fn notify_switch_on_starts_the_watcher_off_stops_it() {
        let bg = TerminalBackground::new();
        let mut cfg = TerminalConfig::default();

        cfg.notify_enabled = true;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.notify_running(), "switching on must start the watcher");
        assert!(!bg.supervisor_running(), "the supervisor switch is still off");

        cfg.notify_enabled = false;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(!bg.notify_running(), "switching off must stop the watcher");
    }

    /// Flipping one switch never disturbs the other — each `reconcile_*`
    /// call only ever touches its own slot.
    #[tokio::test]
    async fn switches_are_independent() {
        let bg = TerminalBackground::new();
        let mut cfg = TerminalConfig::default();
        cfg.supervisor_enabled = true;
        cfg.notify_enabled = true;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.supervisor_running());
        assert!(bg.notify_running());

        cfg.notify_enabled = false;
        bg.reconcile(&cfg, Arc::new(FakeHerdr::new()), store(), None);
        assert!(bg.supervisor_running(), "turning off notify must not touch the supervisor");
        assert!(!bg.notify_running());
    }

    /// Reconciling twice with the same "on" config must not re-spawn (no
    /// observable effect here beyond not panicking/leaking — the `(true,
    /// Some(existing)) => *slot = Some(existing)` arm is what this exercises).
    #[tokio::test]
    async fn reconciling_an_already_running_switch_is_a_no_op() {
        let bg = TerminalBackground::new();
        let mut cfg = TerminalConfig::default();
        cfg.supervisor_enabled = true;
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
}
