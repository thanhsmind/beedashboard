//! Real end-to-end coverage for `waggledance stop` against a stale
//! `daemon.lock` (dls-1). Reported symptom: a lock naming a pid that had
//! already exited and a port nothing listens on made `stop` hang instead of
//! returning promptly. Root cause found by reproduction, not guessed:
//! `health_check`'s `TcpStream::connect` (`crates/waggledance-core/src/daemon.rs`)
//! had no connect timeout — on a network that silently drops packets to a
//! dead port instead of answering with an immediate RST, that connect call
//! blocked indefinitely. The pid-kill step itself was never the slow part.
//!
//! `stop_daemon()` (`crates/waggledance/src/cli.rs`) was also fixed to confirm
//! liveness via `health_check` *before* sending a kill signal by pid — a
//! stale lock's pid can be recycled by an unrelated live process, and
//! killing by pid alone would have signaled that process.

use waggledance_core::daemon::{health_check, DaemonInfo};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Kills and reaps a spawned child on drop, even if an assertion above
/// panics — a leaked process would otherwise strand a listening port/pid
/// across test runs.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn scratch_home(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "waggledance-e2e-stop-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join(".waggledance")).expect("create scratch HOME/.waggledance dir");
    dir
}

fn lock_path(home: &Path) -> PathBuf {
    home.join(".waggledance").join("daemon.lock")
}

fn write_lock(home: &Path, pid: u32, host: &str, port: u16) {
    let info = DaemonInfo {
        pid,
        host: host.to_string(),
        port,
        started_at: "2026-08-06T00:00:00Z".to_string(),
        // This fork records the binary's own version in the lock file; the
        // upstream test this file came from predates that field. A stale lock
        // written by a build that never carried one is exactly the `None` case.
        version: None,
    };
    std::fs::write(
        lock_path(home),
        serde_json::to_vec_pretty(&info).unwrap(),
    )
    .expect("write daemon.lock");
}

/// A pid guaranteed to name no running process: spawn a trivial short-lived
/// child and wait for it to be reaped. Once `wait()` returns, the OS has
/// reclaimed the pid (no zombie left behind), so signaling it fails exactly
/// like the reported "throwaway daemon that had already exited" scenario —
/// without hardcoding a pid number that might collide with something real
/// on the machine running this test.
fn dead_pid() -> u32 {
    let mut child = if cfg!(windows) {
        Command::new("cmd").args(["/C", "exit 0"]).spawn()
    } else {
        Command::new("true").spawn()
    }
    .expect("spawn short-lived helper process");
    let pid = child.id();
    child.wait().expect("wait for short-lived helper to exit");
    pid
}

/// A port nothing is listening on, chosen WITHOUT ever locally binding it.
/// This matters: in this repo's own sandboxed test/agent environment, a
/// `connect()` to a port that was itself just bound-then-released by the
/// same process gets an immediate refusal (normal socket reuse/TIME_WAIT
/// behavior), while a `connect()` to a port that was never locally touched
/// at all is silently dropped instead of refused -- exactly the black hole
/// this cell's fix bounds with a connect timeout, and exactly the shape of
/// the real bug report (a port left behind by a daemon process that had
/// already exited, never touched by the `stop` invocation itself). Probing
/// via bind-then-drop first would silently make the "returns promptly" test
/// pass even without the fix, proving nothing. Distinct fixed values per
/// caller avoid the two dead-port tests in this file colliding with each
/// other when run concurrently.
fn cold_dead_port(offset: u16) -> u16 {
    58_601 + offset
}

fn wait_for_lock(home: &Path, timeout: Duration) -> DaemonInfo {
    let path = lock_path(home);
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(info) = serde_json::from_str::<DaemonInfo>(&text) {
                return info;
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "daemon.lock never appeared/parsed within {timeout:?} at {}",
                path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_health(host: &str, port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if health_check(host, port) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Runs `waggledance <args>` against `home`, returning `None` if it did not exit
/// within `timeout` (killing it so the test doesn't itself hang). This is
/// the reproduction harness: before the fix, `stop` against a stale lock
/// never returns, so this returns `None` and the caller's assertion fails —
/// exactly the observed hang, driven through the real compiled binary.
fn run_with_timeout(bin: &str, args: &[&str], home: &Path, timeout: Duration) -> Option<Output> {
    let mut child = Command::new(bin)
        .args(args)
        .env("HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn waggledance");

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_end(&mut stdout);
                }
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_end(&mut stderr);
                }
                return Some(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// The reproduction: a stale lock naming a dead pid and a dead port must not
/// hang `stop` — it must return within a bounded time and clear the lock.
/// This test failed (timed out) before the connect-timeout fix and passes
/// after it.
#[test]
fn stop_returns_promptly_against_a_stale_lock_and_clears_it() {
    let bin = env!("CARGO_BIN_EXE_waggledance");
    let home = scratch_home("stale");
    let pid = dead_pid();
    let port = cold_dead_port(0);
    write_lock(&home, pid, "127.0.0.1", port);

    let bound = Duration::from_secs(5);
    let start = Instant::now();
    let output = run_with_timeout(bin, &["stop"], &home, bound).unwrap_or_else(|| {
        panic!(
            "waggledance stop did not return within {bound:?} against a stale lock \
             (dead pid {pid}, dead port {port}) -- this is the hang this test reproduces"
        )
    });
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "waggledance stop exited non-zero: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < bound,
        "waggledance stop took {elapsed:?} against a stale lock, expected well under {bound:?}"
    );
    assert!(
        !lock_path(&home).exists(),
        "stale lock should be cleared after stop"
    );
}

/// Edge: no lock file at all still reports no daemon running, not an error,
/// and returns immediately.
#[test]
fn stop_with_no_lock_reports_no_daemon_running() {
    let bin = env!("CARGO_BIN_EXE_waggledance");
    let home = scratch_home("no-lock");

    let output = run_with_timeout(bin, &["stop"], &home, Duration::from_secs(5))
        .expect("waggledance stop did not return promptly with no lock at all");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No daemon running."),
        "expected 'No daemon running.', got: {stdout}"
    );
}

/// Edge: a lock naming a pid that is alive but is not waggledance (e.g. a
/// recycled pid) must not be killed, and must not be treated as a running
/// daemon.
#[cfg(unix)]
#[test]
fn stop_does_not_kill_an_unrelated_live_process_named_by_a_stale_lock() {
    let bin = env!("CARGO_BIN_EXE_waggledance");
    let home = scratch_home("unrelated-pid");

    let unrelated = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn unrelated live process");
    let unrelated_pid = unrelated.id();
    let mut guard = ChildGuard(unrelated);

    // Port is dead, so this pid is alive but is not actually serving waggledance.
    let port = cold_dead_port(1);
    write_lock(&home, unrelated_pid, "127.0.0.1", port);

    let output = run_with_timeout(bin, &["stop"], &home, Duration::from_secs(5))
        .expect("waggledance stop did not return promptly against an unrelated-pid lock");
    assert!(output.status.success());

    // Still alive: try_wait() returns None only while the process is running.
    assert!(
        guard
            .0
            .try_wait()
            .expect("try_wait on unrelated process")
            .is_none(),
        "the unrelated live process (pid {unrelated_pid}) must not have been killed"
    );
    assert!(
        !lock_path(&home).exists(),
        "the stale lock should still be cleared even though nothing was killed"
    );
}

/// Regression: a genuinely live daemon is still stopped and its lock
/// cleared.
#[test]
fn stop_still_stops_a_genuinely_live_daemon() {
    let bin = env!("CARGO_BIN_EXE_waggledance");
    let home = scratch_home("live");

    let child = Command::new(bin)
        .args(["serve", "--port", "0", "--host", "127.0.0.1"])
        .env("HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn waggledance serve");
    let mut guard = ChildGuard(child);

    let info = wait_for_lock(&home, Duration::from_secs(10));
    assert!(
        wait_for_health(&info.host, info.port, Duration::from_secs(10)),
        "daemon never answered /health on {}:{}",
        info.host,
        info.port
    );

    let output = run_with_timeout(bin, &["stop"], &home, Duration::from_secs(5))
        .expect("waggledance stop did not return promptly against a live daemon");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Stopped daemon"),
        "expected a 'Stopped daemon' message, got: {stdout}"
    );

    assert!(
        !lock_path(&home).exists(),
        "the lock should be cleared after stopping a genuinely live daemon"
    );

    // The stop already killed the real daemon; avoid the guard's Drop trying
    // (and failing loudly) to kill an already-reaped process.
    let _ = guard.0.try_wait();
}

/// Regression: the orphaned-daemon guard (cli.rs's `stop_daemon`) still
/// refuses to clear the lock of a daemon that answers but did not die — here
/// simulated by a lock whose pid is dead (so the kill step fails) while its
/// host:port still names a genuinely live daemon.
#[test]
fn stop_keeps_the_lock_when_kill_fails_but_the_daemon_still_answers() {
    let bin = env!("CARGO_BIN_EXE_waggledance");
    let home = scratch_home("orphaned-guard");

    let child = Command::new(bin)
        .args(["serve", "--port", "0", "--host", "127.0.0.1"])
        .env("HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn waggledance serve");
    let guard = ChildGuard(child);

    let info = wait_for_lock(&home, Duration::from_secs(10));
    assert!(
        wait_for_health(&info.host, info.port, Duration::from_secs(10)),
        "daemon never answered /health on {}:{}",
        info.host,
        info.port
    );

    // Overwrite the lock's pid with a dead one, so the kill step fails, while
    // the real daemon (a different, still-live pid) keeps answering on the
    // recorded host:port.
    let dead = dead_pid();
    write_lock(&home, dead, &info.host, info.port);

    let output = run_with_timeout(bin, &["stop"], &home, Duration::from_secs(5))
        .expect("waggledance stop did not return promptly against the orphaned-guard scenario");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Could not stop"),
        "expected a 'Could not stop' message, got: {stdout}"
    );

    assert!(
        lock_path(&home).exists(),
        "the lock must NOT be cleared -- the daemon it names is still answering"
    );
    assert!(
        health_check(&info.host, info.port),
        "the real daemon must still be alive and answering"
    );

    drop(guard);
}
