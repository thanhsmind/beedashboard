//! herdr supervisor — an in-process watchdog that pings herdr and relaunches
//! it when it is down (D1: ported from herdr-go's `supervisor.rs`). NOT
//! external software: a tokio loop inside `waggledance` itself. Restarting herdr
//! recovers workspace/tab/pane structure and relaunches agents via native
//! resume — it does **not** rescue an agent's in-flight work that died with
//! herdr (a herdr limitation, not waggledance's). The supervisor never
//! force-kills anything.
//!
//! Wired in behind the D7 opt-in switch by `crate::TerminalBackground`
//! (`crates/waggledance/src/main.rs`): `reconcile` is the only place a
//! [`Supervisor`] is ever constructed, and a switch left off drives it to
//! spawn nothing.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::herdr::Herdr;

/// An action that (re)starts herdr. Injectable so the loop is testable
/// without spawning a real process.
#[async_trait::async_trait]
pub trait RestartAction: Send + Sync {
    async fn restart(&self) -> anyhow::Result<()>;
}

/// Production restart: spawn `herdr --session <name> server` headless.
pub struct SpawnHerdr {
    pub binary: String,
    pub session: String,
}

/// The herdr binary path, from `WAGGLEDANCE_HERDR_BINARY` when set, else `herdr`
/// on `PATH`. Named in waggledance's own env namespace — herdr-go's
/// `HERDR_GO_HERDR_BINARY` does not travel with the port (D1 retires
/// herdr-go as a separate product, so its env-var namespace retires too).
pub fn herdr_binary_from_env() -> String {
    herdr_binary_from_env_value(std::env::var("WAGGLEDANCE_HERDR_BINARY").ok())
}

fn herdr_binary_from_env_value(value: Option<String>) -> String {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "herdr".into())
}

impl SpawnHerdr {
    fn command(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.binary);
        command
            .arg("--session")
            .arg(&self.session)
            .arg("server")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }
}

#[async_trait::async_trait]
impl RestartAction for SpawnHerdr {
    async fn restart(&self) -> anyhow::Result<()> {
        // Detached headless server; the supervisor re-pings to confirm recovery.
        self.command().spawn()?;
        Ok(())
    }
}

/// Health of the supervised runtime, reported on each transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Up,
    Down,
}

/// The largest wait a persistently-down herdr is ever backed off to — a cap
/// so "back off" never drifts into "stop checking entirely".
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Exponential backoff from `base`, doubling on each consecutive restart
/// (`step` 1, 2, 3, …), capped at [`MAX_BACKOFF`]. `step` is 1-indexed: the
/// first restart backs off by exactly `base` (unchanged from before this
/// existed), the second by `2*base`, and so on — so a herdr that recovers on
/// the very next check behaves exactly as it always has.
fn backoff_for(base: Duration, step: u32) -> Duration {
    let shift = step.saturating_sub(1).min(10); // 2^10 * base already dwarfs MAX_BACKOFF for any sane base
    let scaled = base.checked_mul(1u32 << shift).unwrap_or(MAX_BACKOFF);
    scaled.min(MAX_BACKOFF)
}

pub struct Supervisor {
    control: Arc<dyn Herdr>,
    restart: Arc<dyn RestartAction>,
    interval: Duration,
    backoff: Duration,
    /// Flipped by the owner (`TerminalBackground`) the moment the switch is
    /// turned off. Checked immediately before every restart attempt inside
    /// [`check_once`](Self::check_once) so a switch-off that lands while a
    /// check is already in flight still stops the *next* spawn — cancelling
    /// the task itself only lands at its next `.await` point, which can be
    /// after `check_once` has already decided to restart but before it has
    /// actually done so.
    cancelled: Arc<AtomicBool>,
}

impl Supervisor {
    pub fn new(
        control: Arc<dyn Herdr>,
        restart: Arc<dyn RestartAction>,
        interval: Duration,
        backoff: Duration,
    ) -> Self {
        Self::with_cancel_flag(control, restart, interval, backoff, Arc::new(AtomicBool::new(false)))
    }

    /// Like [`new`](Self::new), but sharing a caller-owned cancellation flag
    /// instead of a fresh one — the owner flips it to stop the very next
    /// spawn without waiting for cancellation to land at an `.await` point.
    pub fn with_cancel_flag(
        control: Arc<dyn Herdr>,
        restart: Arc<dyn RestartAction>,
        interval: Duration,
        backoff: Duration,
        cancelled: Arc<AtomicBool>,
    ) -> Self {
        Supervisor {
            control,
            restart,
            interval,
            backoff,
            cancelled,
        }
    }

    /// One health check. When down, fire the restart action (after which the
    /// next check confirms recovery) — unless the switch was turned off in
    /// the meantime, in which case the restart is skipped even though health
    /// is still reported `Down`. Returns the observed health and whether a
    /// restart was attempted.
    pub async fn check_once(&self) -> (Health, bool) {
        match self.control.ping().await {
            Ok(_) => (Health::Up, false),
            Err(_) => {
                // Check cancellation immediately before spawning — the last
                // possible moment, and strictly before the one action here
                // with an external side effect (`crates/waggledance/src/main.rs`,
                // this cell's trace).
                if self.cancelled.load(Ordering::SeqCst) {
                    return (Health::Down, false);
                }
                let _ = self.restart.restart().await;
                (Health::Down, true)
            }
        }
    }

    /// Run the supervision loop. `ticks` counts every completed health
    /// check (a real, externally-observable side effect of the loop still
    /// running) — the loop stopping is provable by this counter no longer
    /// advancing, not merely by the caller's own bookkeeping having emptied
    /// a slot (`crates/waggledance/src/main.rs`'s `TerminalBackground`, this
    /// cell's trace). `on_transition` fires only when health flips
    /// (up→down or down→up); `on_backoff` fires on every restart attempt,
    /// reporting the 1-indexed consecutive-restart step and the wait about
    /// to be applied, so a persistently down herdr is reported at every
    /// backoff step rather than exactly once ever.
    pub async fn run<F, B>(self, ticks: Arc<AtomicU64>, mut on_transition: F, mut on_backoff: B)
    where
        F: FnMut(Health) + Send,
        B: FnMut(u32, Duration) + Send,
    {
        let mut last: Option<Health> = None;
        let mut consecutive_restarts: u32 = 0;
        loop {
            let (health, restarted) = self.check_once().await;
            ticks.fetch_add(1, Ordering::Relaxed);
            if last != Some(health) {
                on_transition(health);
                last = Some(health);
            }
            let wait = if restarted {
                consecutive_restarts = consecutive_restarts.saturating_add(1);
                let wait = backoff_for(self.backoff, consecutive_restarts);
                on_backoff(consecutive_restarts, wait);
                wait
            } else {
                consecutive_restarts = 0;
                self.interval
            };
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::herdr::fake::FakeHerdr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingRestart {
        count: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl RestartAction for CountingRestart {
        async fn restart(&self) -> anyhow::Result<()> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn up_herdr_does_not_restart() {
        let fake = Arc::new(FakeHerdr::new());
        let count = Arc::new(AtomicUsize::new(0));
        let sup = Supervisor::new(
            fake,
            Arc::new(CountingRestart {
                count: count.clone(),
            }),
            Duration::from_millis(10),
            Duration::from_millis(10),
        );
        let (health, restarted) = sup.check_once().await;
        assert_eq!(health, Health::Up);
        assert!(!restarted);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn down_herdr_triggers_restart() {
        let fake = Arc::new(FakeHerdr::new());
        fake.set_available(false);
        let count = Arc::new(AtomicUsize::new(0));
        let sup = Supervisor::new(
            fake.clone(),
            Arc::new(CountingRestart {
                count: count.clone(),
            }),
            Duration::from_millis(10),
            Duration::from_millis(10),
        );
        let (health, restarted) = sup.check_once().await;
        assert_eq!(health, Health::Down);
        assert!(restarted);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn recovery_after_restart_reports_up() {
        let fake = Arc::new(FakeHerdr::new());
        fake.set_available(false);
        let count = Arc::new(AtomicUsize::new(0));
        let sup = Supervisor::new(
            fake.clone(),
            Arc::new(CountingRestart {
                count: count.clone(),
            }),
            Duration::from_millis(10),
            Duration::from_millis(10),
        );
        assert_eq!(sup.check_once().await.0, Health::Down);
        // Simulate the restart bringing herdr back up.
        fake.set_available(true);
        assert_eq!(sup.check_once().await.0, Health::Up);
    }

    /// Defect (5): a switch-off must stop the *next* spawn even when it
    /// lands after `check_once` has already observed herdr down but before
    /// the restart itself has been made — cancellation via `abort()` alone
    /// only lands at the task's next `.await` point, which is too late to
    /// guarantee this. `check_once` must check its own cancellation flag
    /// immediately before the one line with an external side effect.
    #[tokio::test]
    async fn cancelling_before_check_once_skips_the_spawn() {
        let fake = Arc::new(FakeHerdr::new());
        fake.set_available(false);
        let count = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sup = Supervisor::with_cancel_flag(
            fake,
            Arc::new(CountingRestart {
                count: count.clone(),
            }),
            Duration::from_millis(10),
            Duration::from_millis(10),
            cancelled.clone(),
        );
        // The switch is turned off before this poll even starts (the
        // ordinary case cancellation already covers) — proves the flag
        // itself, not timing, is what stops the spawn.
        cancelled.store(true, Ordering::SeqCst);
        let (health, restarted) = sup.check_once().await;
        assert_eq!(health, Health::Down, "health is still reported honestly");
        assert!(!restarted, "a cancelled supervisor must not report a restart");
        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "a cancelled supervisor must never spawn herdr"
        );
    }

    /// Defect (1)'s general shape, at the pure-function level: `backoff_for`
    /// must actually grow across consecutive restarts (not return the same
    /// `base` every time — the historical bug) and must stay capped rather
    /// than growing without bound.
    #[test]
    fn backoff_grows_then_caps() {
        let base = Duration::from_secs(3);
        assert_eq!(backoff_for(base, 1), base, "the first backoff is unchanged from before");
        assert!(backoff_for(base, 2) > backoff_for(base, 1), "must grow on a second consecutive restart");
        assert!(backoff_for(base, 3) > backoff_for(base, 2), "must keep growing on a third");
        assert_eq!(backoff_for(base, 1000), MAX_BACKOFF, "must cap rather than grow without bound");
    }

    /// Defect (6): a persistently unavailable herdr must be retried with
    /// growing spacing, and every backoff step must be reported — not
    /// logged once ever on the first up→down transition and then silently
    /// forever after.
    #[tokio::test]
    async fn persistent_down_backs_off_and_reports_every_step() {
        let fake = Arc::new(FakeHerdr::new());
        fake.set_available(false);
        let sup = Supervisor::new(
            fake,
            Arc::new(CountingRestart {
                count: Arc::new(AtomicUsize::new(0)),
            }),
            Duration::from_millis(2),
            Duration::from_millis(2),
        );
        let steps: Arc<std::sync::Mutex<Vec<(u32, Duration)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let steps_for_run = steps.clone();
        let ticks = Arc::new(AtomicU64::new(0));
        let run = sup.run(
            ticks,
            |_health| {},
            move |step, wait| steps_for_run.lock().unwrap().push((step, wait)),
        );
        // Bounded run: enough backoff steps to prove growth without a test
        // that hangs forever on the supervisor's own infinite loop.
        let _ = tokio::time::timeout(Duration::from_millis(200), run).await;

        let recorded = steps.lock().unwrap().clone();
        assert!(
            recorded.len() >= 3,
            "must report more than one backoff step for a persistently down herdr: {recorded:?}"
        );
        for pair in recorded.windows(2) {
            assert!(
                pair[1].1 >= pair[0].1,
                "backoff must never shrink between consecutive steps: {recorded:?}"
            );
        }
        assert!(
            recorded.iter().any(|(_, wait)| *wait > Duration::from_millis(2)),
            "must actually back off beyond the base interval eventually: {recorded:?}"
        );
    }

    #[test]
    fn production_restart_is_shell_free_and_session_explicit() {
        use std::ffi::OsStr;

        let command = SpawnHerdr {
            binary: "herdr-custom".into(),
            session: "gateway-team".into(),
        }
        .command();
        let command = command.as_std();
        assert_eq!(command.get_program(), OsStr::new("herdr-custom"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                OsStr::new("--session"),
                OsStr::new("gateway-team"),
                OsStr::new("server")
            ]
        );
    }

    #[test]
    fn herdr_binary_env_override_ignores_empty_values() {
        assert_eq!(herdr_binary_from_env_value(None), "herdr");
        assert_eq!(herdr_binary_from_env_value(Some("   ".into())), "herdr");
        assert_eq!(
            herdr_binary_from_env_value(Some(r"D:\a\_temp\herdr.exe".into())),
            r"D:\a\_temp\herdr.exe"
        );
    }
}
