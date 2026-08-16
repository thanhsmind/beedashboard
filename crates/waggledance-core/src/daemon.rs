//! Single-daemon coordination: the `~/.waggledance/daemon.lock` (pid + port) and a
//! health probe. Shared by the CLI/daemon and the desktop shell so every
//! launcher agrees on one server (PRD §7.1/§7.5).

use crate::config::{self, write_atomic};
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

/// Bound on the TCP connect step of `health_check`. `TcpStream::connect` has
/// no connect timeout of its own (only the read/write timeouts set after a
/// connection is established) — on a host/network that silently drops
/// packets to a dead port instead of answering with an immediate RST, that
/// connect call blocks indefinitely. This is what made `waggledance stop`/
/// `restart` hang against a stale lock naming a dead port: the process kill
/// itself is fast, but the orphaned-daemon check re-probes the port via
/// `running_daemon()` → `health_check()`, and that probe never returned.
const HEALTH_CHECK_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub started_at: String,
    /// Version of the binary that started this daemon.
    ///
    /// Optional because lock files written by builds before this field existed
    /// have to keep parsing — a daemon that predates it reads as `None`, which is
    /// itself the signal that it is older than the current binary.
    #[serde(default)]
    pub version: Option<String>,
}

impl DaemonInfo {
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

pub fn lock_path() -> PathBuf {
    config::daemon_lock_path()
}

pub fn write_lock(info: &DaemonInfo) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(info).map_err(|e| crate::error::Error::Other(e.to_string()))?;
    write_atomic(&lock_path(), &bytes)
}

pub fn read_lock() -> Option<DaemonInfo> {
    let text = std::fs::read_to_string(lock_path()).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn remove_lock() {
    let _ = std::fs::remove_file(lock_path());
}

/// The daemon in the lock, but only if it actually answers on its port.
/// A stale lock (process gone) reads as not-running.
pub fn running_daemon() -> Option<DaemonInfo> {
    let info = read_lock()?;
    if health_check(&info.host, info.port) {
        Some(info)
    } else {
        None
    }
}

/// True if `host` is a wildcard bind address (`0.0.0.0` / `::` / `[::]`).
/// Duplicated from `waggledance`'s `runtime.rs::is_wildcard` rather than shared:
/// `waggledance-core` cannot depend on the binary crate (wrong dependency
/// direction), and this is a 2-line pure predicate with no drift risk.
fn is_wildcard(host: &str) -> bool {
    matches!(host, "0.0.0.0" | "::" | "[::]")
}

/// The version the *running* daemon reports on `/health`, not the version in the
/// lock file.
///
/// Upgrading the binary does not restart a daemon that is already serving, so the
/// live process can be older than the CLI that just wrote to the database. That
/// mismatch is invisible until a short link 404s, because the old process has no
/// `/s/` route. `None` means the daemon did not answer, or answered without a
/// version — both of which mean "older than this build" for a caller's purposes.
pub fn daemon_version(host: &str, port: u16) -> Option<String> {
    let body = health_body(host, port)?;
    let key = "\"version\"";
    let start = body.find(key)? + key.len();
    let rest = &body[start..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')? + open;
    Some(rest[open..close].to_string())
}

/// Minimal blocking HTTP GET /health; true if it looks like waggledance.
pub fn health_check(host: &str, port: u16) -> bool {
    match health_body(host, port) {
        Some(buf) => looks_like_daemon(&buf),
        None => false,
    }
}

/// The detection half of the `server.rs` ↔ `daemon.rs` health handshake: true
/// if a raw HTTP response body looks like it came from this daemon's own
/// `/health` route. Pulled out of `health_check` as its own named predicate
/// so a test can point it directly at a served `/health` body from
/// `waggledance`'s `server.rs`, rather than each side asserting against its
/// own hardcoded copy of the string — two copies of the same literal can
/// drift apart in silence exactly the way a plain `contains("\"waggledance\"")`
/// check once did across a rename.
pub fn looks_like_daemon(body: &str) -> bool {
    body.contains("\"waggledance\"") || body.contains("200 OK")
}

/// Raw `GET /health` response text, or `None` if the daemon did not answer.
fn health_body(host: &str, port: u16) -> Option<String> {
    // A wildcard-bound daemon can't be dialed back on its own bind address on
    // every platform (e.g. WSAEADDRNOTAVAIL on macOS/Windows), so connect to
    // loopback instead. Only the connect target changes -- the `Host:` header
    // below keeps using the original, unsubstituted `host`.
    let connect_host = if is_wildcard(host) {
        if host == "0.0.0.0" {
            "127.0.0.1"
        } else {
            "::1"
        }
    } else {
        host
    };
    // Kept from this fork across the upstream split of `health_check` into
    // `health_check`/`health_body`: an unreachable host must fail on our own
    // clock rather than on the platform's connect default, which on a dropped
    // packet is tens of seconds. Only the return shape changes here -- the
    // enclosing function now answers `Option<String>`, so the two early exits
    // become `?` instead of `return false`.
    let addr = format!("{connect_host}:{port}")
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())?;
    let mut stream = TcpStream::connect_timeout(&addr, HEALTH_CHECK_CONNECT_TIMEOUT).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .ok();
    let req = format!("GET /health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = String::new();
    let _ = stream.take(4096).read_to_string(&mut buf);
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_info_serde_roundtrip() {
        let info = DaemonInfo {
            pid: 42,
            host: "127.0.0.1".into(),
            port: 7700,
            started_at: "2026-07-15T00:00:00Z".into(),
            version: Some("0.5.2".into()),
        };
        let s = serde_json::to_string(&info).unwrap();
        let back: DaemonInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(back.pid, 42);
        assert_eq!(back.base_url(), "http://127.0.0.1:7700");
        assert_eq!(back.version.as_deref(), Some("0.5.2"));
    }

    /// A lock file written before `version` existed must still parse — a daemon
    /// from an older build is exactly the case this field exists to detect, so
    /// failing to read its lock would defeat the purpose.
    #[test]
    fn a_lock_file_without_version_still_parses() {
        let legacy =
            r#"{"pid":42,"host":"127.0.0.1","port":7700,"started_at":"2026-07-15T00:00:00Z"}"#;
        let info: DaemonInfo = serde_json::from_str(legacy).unwrap();
        assert_eq!(info.pid, 42);
        assert_eq!(info.version, None);
    }

    #[test]
    fn daemon_version_is_none_when_nothing_answers() {
        assert_eq!(daemon_version("127.0.0.1", 59_998), None);
    }

    #[test]
    fn looks_like_daemon_accepts_the_apps_own_health_body_shape() {
        assert!(looks_like_daemon(
            r#"{"status":"ok","app":"waggledance","version":"0.1.0"}"#
        ));
        assert!(looks_like_daemon("HTTP/1.1 200 OK\r\n\r\n"));
        assert!(!looks_like_daemon(
            r#"{"status":"ok","app":"some-other-app"}"#
        ));
    }

    #[test]
    fn health_check_false_on_dead_port() {
        // Nothing listening on this port → false, no panic.
        assert!(!health_check("127.0.0.1", 59_999));
    }

    #[test]
    fn health_check_returns_promptly_on_a_dead_port() {
        // Root-cause regression: `TcpStream::connect` had no connect timeout
        // (only the read/write timeouts set after a connection is
        // established), so on a network that drops rather than actively
        // refuses packets to a dead port, the connect call blocked
        // indefinitely — this is what made `waggledance stop`/`restart` hang
        // against a stale lock naming a dead port (see cli.rs's
        // `stop_daemon`, and the e2e reproduction in
        // crates/waggledance/tests/e2e_stop_stale_lock.rs). Bound well above the
        // 500ms connect timeout to keep this from being flaky under load.
        let start = std::time::Instant::now();
        assert!(!health_check("127.0.0.1", 59_998));
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "health_check took {:?} against a dead port, expected it to be \
             bounded by its connect timeout",
            start.elapsed()
        );
    }

    #[test]
    fn health_check_dials_loopback_for_wildcard_host() {
        // A daemon bound to a wildcard host ("0.0.0.0") can't be dialed back
        // on that address on every platform (macOS/Windows reject it), so
        // health_check must substitute loopback at the connect call site.
        // This test proves that substitution by listening on 127.0.0.1 only
        // and calling health_check with "0.0.0.0".
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Drain the request so the client's write doesn't block/hang.
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
            }
        });

        assert!(health_check("0.0.0.0", port));

        handle.join().unwrap();
    }
}
