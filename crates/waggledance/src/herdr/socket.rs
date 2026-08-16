//! Socket client for the real herdr server. Speaks `herdr.sock`'s newline-JSON
//! request/response API, **one request per connection** (PBI-001): each call
//! opens the socket, writes one `{id,method,params}\n`, reads one response line,
//! closes. Error responses carry no `id` (correlate by being the sole reply).

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
#[cfg(windows)]
use interprocess::local_socket::tokio::Stream as LocalStream;
#[cfg(windows)]
use interprocess::local_socket::{ConnectOptions, GenericNamespaced, ToNsName};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(unix)]
use tokio::net::UnixStream as LocalStream;

use super::wire::*;
use super::{
    generate_agent_name, retry_on_name_collision, AgentStarted, Herdr, HerdrError, ReadSource,
    Result, TabCreated,
};

/// Default socket path (herdr's per-user runtime socket).
pub fn default_socket_path() -> Result<PathBuf> {
    default_socket_path_from_config_dir(herdr_config_dir())
}

/// Resolve herdr's own per-user config directory. On Windows this needs the
/// roaming AppData root — herdr-go got that from its own
/// `crate::config::native_roaming_app_data()`, an edge this port must not
/// reach for (waggledance-core has no equivalent, and must not gain one just to
/// serve this client). The Windows resolution is instead injected as a
/// parameter into [`herdr_config_dir_with`], the same seam
/// `default_socket_path_from_config_dir`/`resolve_socket_path_from_config_dir`
/// already use below, with [`windows_roaming_app_data`] as the real default.
fn herdr_config_dir() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        herdr_config_dir_with(windows_roaming_app_data)
    }
    #[cfg(not(windows))]
    {
        Ok(std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config/herdr"))
    }
}

/// Pure: `herdr_config_dir`'s Windows branch, with the roaming AppData root
/// injected rather than looked up — testable with a fake root, and the only
/// thing that ever needs waggledance-core to know what "roaming AppData" means.
#[cfg(windows)]
fn herdr_config_dir_with(
    roaming_app_data: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<PathBuf> {
    roaming_app_data()
        .map(|base| base.join("herdr"))
        .map_err(|error| {
            HerdrError::Unavailable(format!(
                "native Windows roaming application data is unavailable; cannot resolve herdr endpoint ({error})"
            ))
        })
}

/// The real Windows roaming AppData root (`%APPDATA%`) — the injected
/// default for [`herdr_config_dir_with`].
#[cfg(windows)]
fn windows_roaming_app_data() -> std::io::Result<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "APPDATA is unavailable; cannot resolve the Windows roaming application data directory",
            )
        })
}

fn default_socket_path_from_config_dir(config_dir: Result<PathBuf>) -> Result<PathBuf> {
    config_dir.map(|base| base.join("herdr.sock"))
}

/// Resolve the logical herdr endpoint shared by normal startup and doctor.
/// An explicit socket override wins, then a named session, then the historical
/// default endpoint. The logical filesystem path is retained on Windows because
/// herdr also uses it for its ownership marker.
///
/// Only exercised by this module's own tests today — every production caller
/// builds a `SocketHerdr` from a path it already has — kept `#[cfg(test)]`
/// rather than deleted since the doc comment above still documents the
/// intended resolution order for a future production caller.
#[cfg(test)]
pub fn resolve_socket_path(explicit: &str, session: &str) -> Result<PathBuf> {
    if !explicit.is_empty() {
        return Ok(PathBuf::from(explicit));
    }
    resolve_socket_path_from_config_dir(explicit, session, herdr_config_dir())
}

#[cfg(test)]
fn resolve_socket_path_from_config_dir(
    explicit: &str,
    session: &str,
    config_dir: Result<PathBuf>,
) -> Result<PathBuf> {
    if !explicit.is_empty() {
        return Ok(PathBuf::from(explicit));
    }
    if session.is_empty() || session == "default" {
        return default_socket_path_from_config_dir(config_dir);
    }
    if !session
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || session == "."
        || session == ".."
    {
        return Err(HerdrError::Unavailable(
            "invalid herdr session name; use letters, digits, '.', '-' or '_'".into(),
        ));
    }
    let default = default_socket_path_from_config_dir(config_dir)?;
    let root = default
        .parent()
        .ok_or_else(|| HerdrError::Unavailable("herdr endpoint has no parent directory".into()))?;
    Ok(root.join("sessions").join(session).join("herdr.sock"))
}

#[cfg(windows)]
fn windows_endpoint_name(path: &Path) -> Result<interprocess::local_socket::Name<'_>> {
    path.as_os_str()
        .to_ns_name::<GenericNamespaced>()
        .map_err(|e| HerdrError::Unavailable(format!("invalid Windows herdr endpoint ({e})")))
}

async fn connect_local(path: &Path) -> Result<LocalStream> {
    #[cfg(unix)]
    {
        return LocalStream::connect(path)
            .await
            .map_err(|e| unavailable_connect_error(&e));
    }

    #[cfg(windows)]
    {
        const ERROR_PIPE_BUSY: i32 = 231;
        const ATTEMPTS: usize = 20;
        for attempt in 0..ATTEMPTS {
            let options = ConnectOptions::new().name(windows_endpoint_name(path)?);
            match options.connect_tokio().await {
                Ok(client) => return Ok(client),
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempt + 1 < ATTEMPTS => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(unavailable_connect_error(&e)),
            }
        }
        unreachable!("bounded named-pipe connection loop always returns")
    }
}

fn unavailable_connect_error(error: &std::io::Error) -> HerdrError {
    let reason = match error.kind() {
        std::io::ErrorKind::NotFound => "endpoint not found; start herdr for this session",
        std::io::ErrorKind::PermissionDenied => "endpoint access denied",
        std::io::ErrorKind::ConnectionRefused => "endpoint refused the connection",
        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe => {
            "endpoint closed the connection"
        }
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "endpoint remained busy",
        _ => "could not connect to endpoint",
    };
    HerdrError::Unavailable(format!("{reason} ({error})"))
}

/// The settle-wait policy `wait_for_pane_to_settle` runs on -- production
/// always gets `SocketHerdr::SETTLE_MIN_QUIET`/`SETTLE_POLL_INTERVAL`/
/// `SETTLE_MAX_WAIT` (see `SocketHerdr::new`); tests substitute
/// millisecond-scale stand-ins via `SocketHerdr::with_settle_durations_for_test`
/// so the real polling logic in `send_input` can be proven through a real
/// mock socket server without spending the real 250ms/1.5s wall-clock time
/// per test.
#[derive(Clone, Copy)]
struct SettleDurations {
    min_quiet: Duration,
    poll_interval: Duration,
    max_wait: Duration,
}

/// A herdr client bound to one socket path.
#[derive(Clone)]
pub struct SocketHerdr {
    path: PathBuf,
    counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
    settle: SettleDurations,
}

impl SocketHerdr {
    /// Minimum quiet window `wait_for_pane_to_settle` waits before its
    /// first poll -- terminal-attach-submit-race: a slow attachment read
    /// (the web Send composing `"prompt\n/path/to/img.png"`) has not even
    /// started resolving into an `[Image #1]` chip yet at t=0, so polling
    /// immediately would see a not-yet-changed screen and declare it
    /// settled before the composer's own redraw has begun. 250ms is enough
    /// for that redraw to have started and bumped the screen's revision --
    /// live-tested against a real Claude Code pane.
    const SETTLE_MIN_QUIET: Duration = Duration::from_millis(250);
    /// Poll interval once the quiet window has elapsed (see
    /// `SETTLE_MIN_QUIET`).
    const SETTLE_POLL_INTERVAL: Duration = Duration::from_millis(100);
    /// Hard cap on the whole settle wait, measured from the text write, not
    /// from the first poll -- past this the Enter is sent regardless of
    /// what the last poll saw. A user's submit is never dropped because the
    /// screen would not hold still.
    const SETTLE_MAX_WAIT: Duration = Duration::from_millis(1500);

    pub fn new(path: PathBuf) -> Self {
        SocketHerdr {
            path,
            counter: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            settle: SettleDurations {
                min_quiet: Self::SETTLE_MIN_QUIET,
                poll_interval: Self::SETTLE_POLL_INTERVAL,
                max_wait: Self::SETTLE_MAX_WAIT,
            },
        }
    }

    /// Test-only seam (see `SettleDurations`'s own doc): every production
    /// caller goes through `new` and gets the real `SETTLE_*` consts --
    /// this exists only so a test can exercise the real `send_input`/
    /// `wait_for_pane_to_settle` polling logic against a real mock socket
    /// server at millisecond scale instead of the real 250ms/1.5s.
    #[cfg(all(test, unix))]
    fn with_settle_durations_for_test(
        path: PathBuf,
        min_quiet: Duration,
        poll_interval: Duration,
        max_wait: Duration,
    ) -> Self {
        SocketHerdr {
            path,
            counter: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            settle: SettleDurations {
                min_quiet,
                poll_interval,
                max_wait,
            },
        }
    }

    fn next_id(&self) -> String {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("gw-{n}")
    }

    /// Waits for `pane_id`'s screen to stop changing before the caller
    /// issues the submit Enter -- see `Herdr::send_input`'s doc for the
    /// fault this closes (terminal-attach-submit-race). Polls
    /// `read_pane(.., ReadSource::Visible, ..)` after an initial quiet
    /// window (`self.settle.min_quiet`) every `self.settle.poll_interval`
    /// until two consecutive reads report the same screen TEXT, or
    /// `self.settle.max_wait` (from this call's own start, i.e. from the
    /// text write) elapses.
    ///
    /// Compares `ScreenRead.text`, never `ScreenRead.revision`
    /// (terminal-attach-submit-race-2): measured against the real herdr
    /// daemon, `revision` is a dead field -- 8 consecutive `pane.read`
    /// calls taken while a Claude Code pane was actively streaming output
    /// all answered `revision: 0` while the screen text visibly changed
    /// underneath them, and every pane in `herdr pane list` reports
    /// revision 0 regardless of activity. A revision-only settle check
    /// would see an "equal" revision on its very first poll and return
    /// immediately, collapsing this wait into a fixed `min_quiet + one
    /// poll_interval` floor no matter whether the screen has actually
    /// stopped changing -- do not "simplify" this back to comparing
    /// `revision` alone; if it is ever reintroduced as a short-circuit,
    /// text equality must still be required alongside it.
    ///
    /// Never returns an error: a poll read failure, any `HerdrError` from
    /// the poll, or the cap all fall through to the same outcome -- give up
    /// waiting and let the caller send the Enter anyway. A user's submit
    /// must never be silently dropped for a screen that will not hold
    /// still.
    async fn wait_for_pane_to_settle(&self, pane_id: &str) {
        let deadline = tokio::time::Instant::now() + self.settle.max_wait;
        tokio::time::sleep(self.settle.min_quiet).await;

        let mut last_text: Option<String> = None;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return;
            }
            let read = match self.read_pane(pane_id, ReadSource::Visible, 0).await {
                Ok(read) => read,
                Err(_) => return,
            };
            if last_text.as_deref() == Some(read.text.as_str()) {
                return;
            }
            last_text = Some(read.text);

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return;
            }
            tokio::time::sleep(self.settle.poll_interval.min(remaining)).await;
        }
    }

    /// One request → one response, on a fresh connection. Returns the `result`
    /// value, or a typed error for an `error` response / transport failure.
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut stream = connect_local(&self.path).await?;

        let req = Request {
            id: self.next_id(),
            method,
            params,
        };
        let mut line = serde_json::to_vec(&req).map_err(|e| HerdrError::Request(e.to_string()))?;
        line.push(b'\n');
        stream
            .write_all(&line)
            .await
            .map_err(|e| HerdrError::Request(e.to_string()))?;
        stream
            .flush()
            .await
            .map_err(|e| HerdrError::Request(e.to_string()))?;

        // Read until the first newline (one response per connection).
        let mut buf = Vec::with_capacity(4096);
        let mut byte = [0u8; 1];
        loop {
            let n = stream
                .read(&mut byte)
                .await
                .map_err(|e| HerdrError::Request(e.to_string()))?;
            if n == 0 {
                break; // EOF
            }
            if byte[0] == b'\n' {
                break;
            }
            buf.push(byte[0]);
            if buf.len() > 8 * 1024 * 1024 {
                return Err(HerdrError::Malformed("response too large".into()));
            }
        }
        parse_response(&buf)
    }

    /// One `agent.start` attempt with an exact, caller-supplied `name` --
    /// no retry. `agent_start` (the trait method) is the public entry point
    /// that owns the collision retry; this is the "try once" it drives.
    async fn agent_start_named(
        &self,
        name: &str,
        workspace_id: &str,
        cwd: Option<&str>,
        argv: &[String],
    ) -> Result<AgentStarted> {
        if argv.is_empty() {
            return Err(HerdrError::InvalidAgentArgv(
                "argv must not be empty".into(),
            ));
        }
        let result = self
            .call(
                "agent.start",
                agent_start_params(name, argv, cwd, workspace_id),
            )
            .await
            .map_err(|e| attach_agent_start_context(e, name, workspace_id))?;
        // result: { "type":"agent_started", "agent": { ..., "pane_id":..., "tab_id":... }, "argv":[...] }
        let agent = result
            .get("agent")
            .ok_or_else(|| HerdrError::Malformed("agent_started.agent missing".into()))?;
        let tab_id = agent
            .get("tab_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HerdrError::Malformed("agent_started.agent.tab_id missing".into()))?
            .to_string();
        let pane_id = agent
            .get("pane_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HerdrError::Malformed("agent_started.agent.pane_id missing".into()))?
            .to_string();
        Ok(AgentStarted {
            tab_id,
            pane_id,
            name: name.to_string(),
        })
    }
}

/// Extract the `result` from a response line, or map an `error` / bad shape to a
/// typed error.
fn parse_response(line: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(line)
        .map_err(|e| HerdrError::Malformed(format!("{e}: {}", String::from_utf8_lossy(line))))?;
    if let Some(result) = v.get("result") {
        return Ok(result.clone());
    }
    if let Some(err) = v.get("error") {
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error")
            .to_string();
        // A missing code is still a real server refusal, not a malformed
        // response -- it maps to Remote with an empty code so the server's
        // own message reaches the operator instead of being replaced by
        // "malformed herdr response".
        let code = err
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        return Err(match code.as_str() {
            // The caller-supplied name/workspace_id is attached by the
            // create methods that own those calls (later cells) -- parsing
            // them out of herdr's human-readable message text would be
            // brittle, so they start empty here. The server's own message
            // (which, for agent_name_taken, enumerates the conflicting
            // terminals) is not thrown away, though -- it rides along so
            // the operator still sees what herdr actually said.
            "agent_name_taken" => HerdrError::AgentNameTaken {
                name: String::new(),
                message,
            },
            "workspace_not_found" => HerdrError::WorkspaceNotFound {
                workspace_id: String::new(),
                message,
            },
            "invalid_agent_argv" => HerdrError::InvalidAgentArgv(message),
            _ => HerdrError::Remote { code, message },
        });
    }
    Err(HerdrError::Malformed(
        "response has neither result nor error".into(),
    ))
}

/// Turn a `session.snapshot` response into a [`Snapshot`].
///
/// Takes the **outer** result value — the same thing `call("session.snapshot")`
/// returns, i.e. `{ "type": ..., "snapshot": { ... } }` — so this is the live
/// extraction path itself, not a parallel copy of it. It is pure so it can be
/// tested against a captured envelope; `snapshot()` below does the I/O and
/// nothing else.
fn parse_snapshot(result: &Value) -> Result<Snapshot> {
    let snapshot_val = result
        .get("snapshot")
        .ok_or_else(|| HerdrError::Malformed("snapshot missing".into()))?;

    // agents[]/panes[]/layouts[] are required by herdr's schema: their absence
    // means a broken or older server, not a normal empty case, so they are hard
    // errors rather than silent empties.
    let required = |field: &str| -> Result<Value> {
        snapshot_val
            .get(field)
            .cloned()
            .ok_or_else(|| HerdrError::Malformed(format!("snapshot.{field} missing")))
    };
    let agents: Vec<Agent> = serde_json::from_value(required("agents")?)
        .map_err(|e| HerdrError::Malformed(e.to_string()))?;
    let panes: Vec<Pane> = serde_json::from_value(required("panes")?)
        .map_err(|e| HerdrError::Malformed(e.to_string()))?;
    let layouts: Vec<PaneLayout> = serde_json::from_value(required("layouts")?)
        .map_err(|e| HerdrError::Malformed(e.to_string()))?;

    // workspaces[]/tabs[] are resolved best-effort: missing or malformed
    // falls back to an empty list rather than failing the whole snapshot.
    let workspaces: Vec<Workspace> = snapshot_val
        .get("workspaces")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let tabs: Vec<Tab> = snapshot_val
        .get("tabs")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let focused = |field: &str| -> Option<String> {
        snapshot_val
            .get(field)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };

    Ok(Snapshot {
        agents,
        workspaces,
        tabs,
        panes,
        layouts,
        focused_workspace_id: focused("focused_workspace_id"),
        focused_tab_id: focused("focused_tab_id"),
        focused_pane_id: focused("focused_pane_id"),
    })
}

/// Build the `tab.create` params — `workspace_id` and `focus: false`,
/// plus `cwd` only when the caller supplied one. Nothing else: no `label`, no
/// `env`. When `cwd` is `None` the key is **omitted entirely** (not an empty
/// string, not null), letting herdr resolve the workspace anchor. Pure so it
/// is testable without a socket, the same seam `parse_snapshot` cut for
/// `session.snapshot`.
fn tab_create_params(workspace_id: &str, cwd: Option<&str>) -> Value {
    let mut params = json!({
        "workspace_id": workspace_id,
        "focus": false,
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    params
}

/// `parse_response` cannot know which workspace the caller asked for, so it
/// leaves `WorkspaceNotFound.workspace_id` empty and defers filling it in to
/// "the create methods that own those calls" (see the comment there) -- this
/// is that method. Every other variant passes through unchanged.
fn attach_workspace_id(error: HerdrError, workspace_id: &str) -> HerdrError {
    match error {
        HerdrError::WorkspaceNotFound { message, .. } => HerdrError::WorkspaceNotFound {
            workspace_id: workspace_id.to_string(),
            message,
        },
        other => other,
    }
}

/// Build the `agent.start` params -- `name`, `argv`, `workspace_id`,
/// `focus: false`, plus `cwd` only when the caller supplied one. Deliberately
/// no `tab_id`/`split`: sending both a tab and a workspace opens
/// `agent_placement_conflict` for no product gain, and a phone has no concept
/// of split direction, so upstream's default placement (split Right off the
/// workspace's active tab) is accepted as-is. When `cwd` is `None` the key is
/// **omitted entirely** -- but unlike `tab.create`, herdr then falls back to
/// its own process directory, not the workspace anchor (see
/// [`super::Herdr::agent_start`]); callers must not omit it unless that is
/// intended. Pure, same testable seam as `tab_create_params`.
fn agent_start_params(name: &str, argv: &[String], cwd: Option<&str>, workspace_id: &str) -> Value {
    let mut params = json!({
        "name": name,
        "argv": argv,
        "workspace_id": workspace_id,
        "focus": false,
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    params
}

/// `parse_response` cannot know the caller-supplied name or workspace_id, so
/// it leaves `AgentNameTaken.name` and `WorkspaceNotFound.workspace_id`
/// empty -- `agent_start_named` (the only caller of `agent.start`) fills
/// them in here. Every other variant passes through unchanged, the same
/// contract as `attach_workspace_id`.
fn attach_agent_start_context(error: HerdrError, name: &str, workspace_id: &str) -> HerdrError {
    match error {
        HerdrError::AgentNameTaken { message, .. } => HerdrError::AgentNameTaken {
            name: name.to_string(),
            message,
        },
        HerdrError::WorkspaceNotFound { message, .. } => HerdrError::WorkspaceNotFound {
            workspace_id: workspace_id.to_string(),
            message,
        },
        other => other,
    }
}

#[async_trait]
impl Herdr for SocketHerdr {
    async fn snapshot(&self) -> Result<Snapshot> {
        let result = self.call("session.snapshot", json!({})).await?;
        parse_snapshot(&result)
    }

    async fn ping(&self) -> Result<ProtocolInfo> {
        let result = self.call("ping", json!({})).await?;
        let protocol = result
            .get("protocol")
            .and_then(|p| p.as_u64())
            .ok_or_else(|| HerdrError::Malformed("ping.protocol missing".into()))?
            as u32;
        let version = result
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let info = ProtocolInfo {
            protocol,
            server_version: version,
        };
        if !info.is_compatible() {
            return Err(HerdrError::ProtocolMismatch {
                expected: HERDR_PROTOCOL,
                actual: info.protocol,
            });
        }
        Ok(info)
    }

    async fn read_pane(
        &self,
        pane_id: &str,
        source: ReadSource,
        lines: usize,
    ) -> Result<ScreenRead> {
        let mut params =
            json!({ "pane_id": pane_id, "source": source.as_wire(), "format": "ansi" });
        // herdr ignores `lines` for `visible` -- only send it for `recent`,
        // capped at herdr's own 1000-line server-side limit.
        if source == ReadSource::Recent {
            params["lines"] = json!(lines.min(1000));
        }
        let result = self.call("pane.read", params).await?;
        // result: { "type":"pane_read", "read": { "text":..., "revision":... } }
        let read = result
            .get("read")
            .ok_or_else(|| HerdrError::Malformed("pane_read.read missing".into()))?;
        let text = read
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        let revision = read.get("revision").and_then(|r| r.as_u64()).unwrap_or(0);
        Ok(ScreenRead { text, revision })
    }

    async fn send_input(&self, pane_id: &str, text: &str, submit: bool) -> Result<()> {
        if !text.is_empty() {
            self.call(
                "pane.send_input",
                json!({ "pane_id": pane_id, "text": text }),
            )
            .await?;
            if submit {
                // terminal-attach-submit-race: let the composer settle
                // before the Enter lands -- see `wait_for_pane_to_settle`.
                self.wait_for_pane_to_settle(pane_id).await;
            }
        }
        if submit {
            // Send≠submit: a separate Enter key submits the composer.
            self.call(
                "pane.send_input",
                json!({ "pane_id": pane_id, "keys": ["enter"] }),
            )
            .await?;
        }
        Ok(())
    }

    async fn send_keys(&self, pane_id: &str, keys: &[String]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        self.call(
            "pane.send_keys",
            json!({ "pane_id": pane_id, "keys": keys }),
        )
        .await?;
        Ok(())
    }

    async fn send_text(&self, pane_id: &str, bytes: &str) -> Result<()> {
        // Raw byte passthrough: unlike send_input/send_keys, herdr's
        // `pane.send_text` handler delivers the exact bytes with no
        // bracketed-paste wrapping and no named-key translation -- the only
        // channel that can send a literal VT escape sequence.
        self.call(
            "pane.send_text",
            json!({ "pane_id": pane_id, "text": bytes }),
        )
        .await?;
        Ok(())
    }

    async fn tab_create(&self, workspace_id: &str, cwd: Option<&str>) -> Result<TabCreated> {
        let result = self
            .call("tab.create", tab_create_params(workspace_id, cwd))
            .await
            .map_err(|e| attach_workspace_id(e, workspace_id))?;
        // result: { "type":"tab_created", "tab": { "tab_id":..., ... }, "root_pane": { "pane_id":..., ... } }
        let tab_id = result
            .get("tab")
            .and_then(|t| t.get("tab_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| HerdrError::Malformed("tab_created.tab.tab_id missing".into()))?
            .to_string();
        let pane_id = result
            .get("root_pane")
            .and_then(|p| p.get("pane_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| HerdrError::Malformed("tab_created.root_pane.pane_id missing".into()))?
            .to_string();
        Ok(TabCreated { tab_id, pane_id })
    }

    async fn agent_start(
        &self,
        workspace_id: &str,
        cwd: Option<&str>,
        argv: &[String],
    ) -> Result<AgentStarted> {
        retry_on_name_collision(generate_agent_name, |name| async move {
            self.agent_start_named(&name, workspace_id, cwd, argv).await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tracked capture of the INNER `snapshot` object (herdr 0.7.4, protocol
    /// 16). `parse_snapshot` takes the OUTER value, so tests wrap it — the
    /// wrapping belongs to the test, never to `parse_snapshot`, which must keep
    /// matching what `call("session.snapshot")` actually returns.
    const LIVE_SNAPSHOT: &str = include_str!("testdata/live-snapshot.json");

    fn live_envelope() -> Value {
        let inner: Value = serde_json::from_str(LIVE_SNAPSHOT).unwrap();
        json!({ "type": "session_snapshot", "snapshot": inner })
    }

    #[test]
    fn envelope_socket_parse_populates_new_arrays() {
        // The live extraction path builds Snapshot by hand, so an empty panes[]
        // would compile and pass every serde fixture test. This exercises that
        // exact path against a real captured envelope.
        let snap = parse_snapshot(&live_envelope()).unwrap();

        assert_eq!(snap.agents.len(), 7);
        assert_eq!(snap.panes.len(), 8, "panes[] must not arrive empty");
        assert_eq!(snap.layouts.len(), 5, "layouts[] must not arrive empty");
        assert_eq!(snap.workspaces.len(), 5);
        assert_eq!(snap.tabs.len(), 5);

        assert!(snap.workspaces.iter().all(|w| w.active_tab_id.is_some()));
        assert!(snap.layouts.iter().all(|l| l.focused_pane_id.is_some()));
        assert!(snap.panes.iter().all(|p| p.cwd.is_some()));
        assert!(snap.panes.iter().any(|p| p.foreground_cwd.is_some()));

        assert_eq!(snap.focused_workspace_id.as_deref(), Some("wB"));
        assert_eq!(snap.focused_tab_id.as_deref(), Some("wB:t1"));
        assert_eq!(snap.focused_pane_id.as_deref(), Some("wB:p1"));
    }

    #[test]
    fn envelope_socket_parse_rejects_missing_required_arrays() {
        // Required in herdr's schema — absence means a broken or older server,
        // so it is an error here, unlike the best-effort workspaces[]/tabs[].
        let inner: Value = serde_json::from_str(LIVE_SNAPSHOT).unwrap();
        for field in ["agents", "panes", "layouts"] {
            let mut stripped = inner.clone();
            stripped.as_object_mut().unwrap().remove(field);
            assert!(
                matches!(
                    parse_snapshot(&json!({ "snapshot": stripped })),
                    Err(HerdrError::Malformed(_))
                ),
                "missing {field} must be malformed"
            );
        }

        // workspaces[]/tabs[] keep degrading to empty instead of failing.
        let mut stripped = inner.clone();
        stripped.as_object_mut().unwrap().remove("workspaces");
        stripped.as_object_mut().unwrap().remove("tabs");
        let snap = parse_snapshot(&json!({ "snapshot": stripped })).unwrap();
        assert!(snap.workspaces.is_empty());
        assert!(snap.tabs.is_empty());
        assert_eq!(snap.panes.len(), 8);
    }

    #[test]
    fn tabcreate_params_carry_workspace_cwd_and_focus_false() {
        // Exactly workspace_id, cwd, focus:false -- no label, no env.
        let params = tab_create_params("w1", Some("/home/dev/project"));
        assert_eq!(
            params,
            json!({
                "workspace_id": "w1",
                "cwd": "/home/dev/project",
                "focus": false,
            })
        );
        let obj = params.as_object().unwrap();
        assert_eq!(
            obj.len(),
            3,
            "must send exactly these three keys, no label/env"
        );
    }

    #[test]
    fn createcwd_tabcreate_params_omit_cwd_when_none() {
        // With no cwd, the key must be ABSENT -- not "" and not null -- so
        // herdr resolves the workspace anchor.
        let params = tab_create_params("w1", None);
        assert_eq!(
            params,
            json!({
                "workspace_id": "w1",
                "focus": false,
            })
        );
        let obj = params.as_object().unwrap();
        assert!(
            !obj.contains_key("cwd"),
            "cwd key must be omitted, not empty"
        );
        assert_eq!(obj.len(), 2, "exactly workspace_id and focus, no cwd");
    }

    #[test]
    fn createcwd_agentstart_params_omit_cwd_when_none() {
        // Same omit-when-absent contract as tab.create's params -- the key is
        // gone, not blanked. (The asymmetric FALLBACK herdr then applies is a
        // server behavior documented on the trait, not visible in the wire
        // params, which look identical to tab.create's absent-cwd case.)
        let argv = vec!["claude".to_string()];
        let params = agent_start_params("mobile-agent-1", &argv, None, "w1");
        assert_eq!(
            params,
            json!({
                "name": "mobile-agent-1",
                "argv": ["claude"],
                "workspace_id": "w1",
                "focus": false,
            })
        );
        let obj = params.as_object().unwrap();
        assert!(
            !obj.contains_key("cwd"),
            "cwd key must be omitted, not empty"
        );
        assert_eq!(obj.len(), 4, "name, argv, workspace_id, focus -- no cwd");
    }

    #[test]
    fn tabcreate_error_attaches_caller_workspace_id() {
        // parse_response cannot know the workspace the caller asked for, so
        // it hands back an empty workspace_id -- tab_create is the caller
        // that must fill it in before the error reaches the operator.
        let err = HerdrError::WorkspaceNotFound {
            workspace_id: String::new(),
            message: "no active workspace".into(),
        };
        let mapped = attach_workspace_id(err, "w9");
        assert!(matches!(
            mapped,
            HerdrError::WorkspaceNotFound { workspace_id, message }
                if workspace_id == "w9" && message == "no active workspace"
        ));
    }

    #[test]
    fn tabcreate_error_other_variants_pass_through_unchanged() {
        // Only WorkspaceNotFound gets the caller's id attached -- every
        // other variant, including the ones with their own carried data,
        // must be untouched.
        let cases = vec![
            HerdrError::Remote {
                code: "tab_create_failed".into(),
                message: "boom".into(),
            },
            HerdrError::AgentNameTaken {
                name: String::new(),
                message: "name in use".into(),
            },
            HerdrError::InvalidAgentArgv("argv must not be empty".into()),
            HerdrError::Malformed("bad shape".into()),
        ];
        for err in cases {
            let before = err.to_string();
            let mapped = attach_workspace_id(err, "w9");
            assert_eq!(mapped.to_string(), before, "must pass through unchanged");
        }
    }

    #[test]
    fn agentstart_params_omit_tab_id_and_split() {
        // Exactly name, argv, cwd, workspace_id, focus:false -- no tab_id,
        // no split (sending both a tab and a workspace opens
        // agent_placement_conflict for no product gain).
        let argv = vec!["claude".to_string()];
        let params = agent_start_params("mobile-agent-1", &argv, Some("/home/dev/project"), "w1");
        assert_eq!(
            params,
            json!({
                "name": "mobile-agent-1",
                "argv": ["claude"],
                "cwd": "/home/dev/project",
                "workspace_id": "w1",
                "focus": false,
            })
        );
        let obj = params.as_object().unwrap();
        assert_eq!(
            obj.len(),
            5,
            "must send exactly these five keys, no tab_id/split"
        );
        assert!(!obj.contains_key("tab_id"));
        assert!(!obj.contains_key("split"));
    }

    #[test]
    fn agentstart_error_attaches_caller_name_and_workspace_id() {
        // parse_response cannot know the name/workspace the caller asked
        // for, so it hands back both empty -- agent_start_named is the
        // caller that must fill them in before the error reaches the
        // operator.
        let name_taken = HerdrError::AgentNameTaken {
            name: String::new(),
            message: "name in use".into(),
        };
        assert!(matches!(
            attach_agent_start_context(name_taken, "mobile-agent-1", "w9"),
            HerdrError::AgentNameTaken { name, message }
                if name == "mobile-agent-1" && message == "name in use"
        ));

        let ws_not_found = HerdrError::WorkspaceNotFound {
            workspace_id: String::new(),
            message: "no such workspace".into(),
        };
        assert!(matches!(
            attach_agent_start_context(ws_not_found, "mobile-agent-1", "w9"),
            HerdrError::WorkspaceNotFound { workspace_id, message }
                if workspace_id == "w9" && message == "no such workspace"
        ));
    }

    #[test]
    fn agentstart_error_other_variants_pass_through_unchanged() {
        let cases = vec![
            HerdrError::Remote {
                code: "agent_start_failed".into(),
                message: "boom".into(),
            },
            HerdrError::InvalidAgentArgv("argv must not be empty".into()),
            HerdrError::Malformed("bad shape".into()),
        ];
        for err in cases {
            let before = err.to_string();
            let mapped = attach_agent_start_context(err, "mobile-agent-1", "w9");
            assert_eq!(mapped.to_string(), before, "must pass through unchanged");
        }
    }

    #[tokio::test]
    async fn agentstart_empty_argv_errors_without_a_call() {
        // No socket is ever reachable in this test -- if agent_start_named
        // attempted a real call before checking argv, this would hang or
        // fail on connection rather than returning InvalidAgentArgv.
        let client = SocketHerdr::new(PathBuf::from("/nonexistent/herdr.sock"));
        let err = client
            .agent_start_named("mobile-agent-1", "w1", Some("/home/dev"), &[])
            .await
            .unwrap_err();
        assert!(matches!(err, HerdrError::InvalidAgentArgv(_)));
    }

    #[test]
    fn parse_response_extracts_result() {
        let line = br#"{"id":"gw-0","result":{"type":"pong","protocol":16,"version":"0.7.4"}}"#;
        let r = parse_response(line).unwrap();
        assert_eq!(r["protocol"], 16);
    }

    #[test]
    fn parse_response_maps_error() {
        // A coded refusal is a server answer, not a request failure -- this
        // used to collapse into Request, throwing error.code away; that
        // collapse was the defect, so this assertion changed deliberately.
        let line = br#"{"error":{"code":"tab_create_failed","message":"no such pane"}}"#;
        assert!(matches!(
            parse_response(line),
            Err(HerdrError::Remote { code, message })
                if code == "tab_create_failed" && message == "no such pane"
        ));
    }

    #[test]
    fn errcode_agent_name_taken_maps_to_typed_variant() {
        // name starts empty (the caller-supplied name is attached by later
        // cells), but herdr's own message -- which enumerates the
        // conflicting terminals -- must survive, not be discarded.
        let line = br#"{"error":{"code":"agent_name_taken","message":"name in use"}}"#;
        assert!(matches!(
            parse_response(line),
            Err(HerdrError::AgentNameTaken { name, message })
                if name.is_empty() && message == "name in use"
        ));
    }

    #[test]
    fn errcode_workspace_not_found_maps_to_typed_variant() {
        let line = br#"{"error":{"code":"workspace_not_found","message":"no such workspace"}}"#;
        assert!(matches!(
            parse_response(line),
            Err(HerdrError::WorkspaceNotFound { workspace_id, message })
                if workspace_id.is_empty() && message == "no such workspace"
        ));
    }

    #[test]
    fn errcode_invalid_agent_argv_maps_to_typed_variant() {
        let line = br#"{"error":{"code":"invalid_agent_argv","message":"argv must not be empty"}}"#;
        assert!(matches!(
            parse_response(line),
            Err(HerdrError::InvalidAgentArgv(message)) if message == "argv must not be empty"
        ));
    }

    #[test]
    fn errcode_unknown_code_preserved_in_remote() {
        // Every upstream code without a caller that branches on it (e.g.
        // agent_placement_conflict) still reaches the caller with its exact
        // code string intact, not folded into a generic bucket.
        let line = br#"{"error":{"code":"agent_placement_conflict","message":"pane busy"}}"#;
        assert!(matches!(
            parse_response(line),
            Err(HerdrError::Remote { code, message })
                if code == "agent_placement_conflict" && message == "pane busy"
        ));
    }

    #[test]
    fn errcode_missing_code_is_remote_not_malformed() {
        let line = br#"{"error":{"message":"no such pane"}}"#;
        assert!(matches!(
            parse_response(line),
            Err(HerdrError::Remote { code, message })
                if code.is_empty() && message == "no such pane"
        ));
    }

    #[test]
    fn errcode_parse_response_never_produces_request() {
        // parse_response's error branch is the only thing this cell
        // touches, and Request must stay exclusively a local-transport
        // meaning -- never something the error envelope maps to, coded or
        // not. This is the general form of the one assertion
        // parse_response_maps_error deliberately changed above.
        for body in [
            &br#"{"error":{"code":"agent_name_taken","message":"x"}}"#[..],
            &br#"{"error":{"code":"workspace_not_found","message":"x"}}"#[..],
            &br#"{"error":{"code":"invalid_agent_argv","message":"x"}}"#[..],
            &br#"{"error":{"code":"some_unknown_code","message":"x"}}"#[..],
            &br#"{"error":{"message":"x"}}"#[..],
        ] {
            assert!(
                !matches!(parse_response(body), Err(HerdrError::Request(_))),
                "error envelope must never map to Request: {}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn errcode_local_io_failure_still_maps_to_request() {
        // This cell does not touch call()'s serialize/write/flush/read
        // mapping (socket.rs, inside `call`) -- a genuine local transport
        // failure there must still surface as Request, unchanged. Bounded
        // by an outer timeout so a regression here fails fast instead of
        // hanging the suite.
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("herdr.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            let client = SocketHerdr::new(path.clone());

            // Issue the request concurrently with the server closing the
            // connection without ever reading it -- ordering the accept
            // before the call would deadlock (nothing is listening for the
            // client to connect to until accept() is polled), so both run
            // side by side and are joined together.
            let call = tokio::spawn(async move { client.call("ping", json!({})).await });
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream); // peer gone before ever reading the request
            call.await.unwrap()
        })
        .await
        .expect("call must not hang");

        assert!(
            matches!(outcome, Err(HerdrError::Request(_))),
            "expected Request for a closed-peer transport failure, got {outcome:?}"
        );
    }

    /// Millisecond-scale stand-ins for `SocketHerdr::SETTLE_MIN_QUIET`/
    /// `SETTLE_POLL_INTERVAL`/`SETTLE_MAX_WAIT`, fed to
    /// `SocketHerdr::with_settle_durations_for_test` by the settle-wait
    /// tests below -- wide enough (relative to each other) that ordinary
    /// scheduler jitter on a loaded test box cannot collapse the intended
    /// multi-poll window down to zero or one poll, unlike a handful of
    /// single-digit milliseconds would.
    #[cfg(unix)]
    const TEST_SETTLE_MIN_QUIET: Duration = Duration::from_millis(25);
    #[cfg(unix)]
    const TEST_SETTLE_POLL_INTERVAL: Duration = Duration::from_millis(25);
    #[cfg(unix)]
    const TEST_SETTLE_MAX_WAIT: Duration = Duration::from_millis(200);

    /// A mock `herdr.sock` server for the `send_input` settle-wait tests
    /// below: drains every request until the submit Enter arrives (rather
    /// than a fixed count), answering each `pane.read` with `text`
    /// computed by `read_text` so a test can script either "settles after
    /// two identical reads" or "never settles" by choosing that closure.
    /// `revision` is pinned to a constant `0` on every reply -- matching
    /// the real herdr daemon's measured behavior (terminal-attach-submit-
    /// race-2: revision stays 0 across every read while a pane streams)
    /// -- so these tests exercise the settle wait exactly as it behaves
    /// against production, where `revision` alone could never distinguish
    /// "settled" from "still changing". Returns every request it saw, in
    /// arrival order, paired with the `Instant` it was fully received at --
    /// lets a test pin not just what requests arrived but when, e.g. that
    /// the settle wait's first poll did not fire before its own min-quiet
    /// window had actually elapsed.
    #[cfg(unix)]
    async fn run_settle_mock_server(
        listener: tokio::net::UnixListener,
        mut read_text: impl FnMut() -> String + Send + 'static,
    ) -> Vec<(Value, tokio::time::Instant)> {
        let mut requests = Vec::new();
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            loop {
                let n = stream.read(&mut byte).await.unwrap();
                if n == 0 || byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            let received_at = tokio::time::Instant::now();
            let value: Value = serde_json::from_slice(&buf).unwrap();
            let is_submit_enter = value["params"]["keys"] == json!(["enter"]);
            let result = if value["method"] == "pane.read" {
                json!({ "type": "pane_read", "read": { "text": read_text(), "revision": 0 } })
            } else {
                json!({})
            };
            requests.push((value, received_at));
            let mut line = serde_json::to_vec(&json!({ "id": "gw-0", "result": result })).unwrap();
            line.push(b'\n');
            stream.write_all(&line).await.unwrap();
            stream.flush().await.unwrap();
            if is_submit_enter {
                return requests;
            }
        }
    }

    /// agent-terminal-11 / terminal-attach-submit-race: pins the send≠submit
    /// separation at the transport layer, not only against `FakeHerdr`
    /// (which fuses text+submit into one write by appending the newline
    /// itself and bumping the revision once — see its own `send_input`).
    /// The real herdr wraps sent text in bracketed paste, so a client that
    /// collapsed these into a single `pane.send_input` call carrying both
    /// `text` and `keys` would leave every reply sitting unsent in the
    /// composer while the screen poll shows it there — a silent failure of
    /// the whole feature. Proven against a real mock socket server:
    /// `submit: true` with non-empty text must reach the socket as the
    /// FIRST request carrying only the text and the LAST carrying only the
    /// enter keypress -- with the settle wait's own `pane.read` poll (added
    /// by terminal-attach-submit-race) sitting between them, not fused into
    /// either write. Uses `SocketHerdr::with_settle_durations_for_test`
    /// (millisecond-scale stand-ins for the real 250ms/100ms/1.5s policy)
    /// so the real settle-wait logic in `send_input` runs against a real
    /// mock socket server at effectively no wall-clock cost.
    #[cfg(unix)]
    #[tokio::test]
    async fn sendinput_with_submit_issues_two_distinct_socket_requests() {
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("herdr.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            let client = SocketHerdr::with_settle_durations_for_test(
                path.clone(),
                TEST_SETTLE_MIN_QUIET,
                TEST_SETTLE_POLL_INTERVAL,
                TEST_SETTLE_MAX_WAIT,
            );

            // Settles after its second identical read, like a composer
            // whose redraw has already finished by the time the poll loop
            // catches up. Constant text (not just a constant revision)
            // is what actually settles this -- see `run_settle_mock_server`.
            let server = tokio::spawn(run_settle_mock_server(listener, || "steady".to_string()));

            client.send_input("w1:p1", "hello", true).await.unwrap();
            server.await.unwrap()
        })
        .await
        .expect("send_input must not hang")
        .into_iter()
        .map(|(value, _)| value)
        .collect::<Vec<Value>>();

        let first = outcome.first().expect("at least the text request");
        let last = outcome.last().expect("at least the enter request");

        assert_eq!(first["method"], "pane.send_input");
        assert_eq!(first["params"]["text"], "hello");
        assert!(
            first["params"].get("keys").is_none(),
            "the first request must carry only the text, not the enter keypress: {first:?}"
        );

        assert_eq!(last["method"], "pane.send_input");
        assert_eq!(last["params"]["keys"], json!(["enter"]));
        assert!(
            last["params"].get("text").is_none(),
            "the last request must carry only the enter keypress, not the text: {last:?}"
        );

        assert!(
            outcome[1..outcome.len() - 1]
                .iter()
                .any(|r| r["method"] == "pane.read"),
            "at least one pane.read must sit between the text write and the submit Enter, \
             proving the settle wait ran: {outcome:?}"
        );
    }

    /// terminal-attach-submit-race: a pane whose screen never stops
    /// changing (every `pane.read` answers new text) must not withhold
    /// the submit Enter forever -- `wait_for_pane_to_settle`'s
    /// `SETTLE_MAX_WAIT` cap must fire and the Enter must still reach the
    /// socket, proving a user's submit is never silently dropped for a
    /// screen that will not hold still. Uses
    /// `SocketHerdr::with_settle_durations_for_test` (see the other
    /// settle-wait test's doc) so the cap this proves costs single-digit
    /// milliseconds, not the real 1.5s.
    #[cfg(unix)]
    #[tokio::test]
    async fn sendinput_still_sends_enter_when_screen_never_settles() {
        let (elapsed, outcome) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("herdr.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            let client = SocketHerdr::with_settle_durations_for_test(
                path.clone(),
                TEST_SETTLE_MIN_QUIET,
                TEST_SETTLE_POLL_INTERVAL,
                TEST_SETTLE_MAX_WAIT,
            );

            let mut frame = 0u64;
            let server = tokio::spawn(run_settle_mock_server(listener, move || {
                frame += 1;
                format!("frame-{frame}")
            }));

            let started = tokio::time::Instant::now();
            client.send_input("w1:p1", "hello", true).await.unwrap();
            let elapsed = started.elapsed();
            let outcome: Vec<Value> = server
                .await
                .unwrap()
                .into_iter()
                .map(|(value, _)| value)
                .collect();
            (elapsed, outcome)
        })
        .await
        .expect("send_input must not hang even when the screen never settles");

        // A cap-magnitude regression (e.g. the cap silently multiplied or
        // dropped) must not slide under this test's 5s outer timeout --
        // pin the cap's own order of magnitude directly.
        assert!(
            elapsed < TEST_SETTLE_MAX_WAIT * 3,
            "the settle-wait cap must fire near {:?}, not balloon past it -- \
             send_input took {elapsed:?}",
            TEST_SETTLE_MAX_WAIT
        );

        let last = outcome.last().expect("at least the enter request");
        assert_eq!(
            last["method"], "pane.send_input",
            "the cap must still let the Enter through: {outcome:?}"
        );
        assert_eq!(
            last["params"]["keys"],
            json!(["enter"]),
            "the submit must never be dropped for an unsettled screen: {outcome:?}"
        );
        assert!(
            outcome.iter().any(|r| r["method"] == "pane.read"),
            "the settle-wait poll must have run at least once before the cap sent \
             the Enter anyway: {outcome:?}"
        );
    }

    /// terminal-attach-submit-race-2: `run_settle_mock_server` pins
    /// `revision` to a constant `0` on every reply, matching the real
    /// herdr daemon's measured behavior. Against the old revision-only
    /// settle check this pane would falsely "settle" on the very first
    /// poll (revision 0 == revision 0) regardless of what the screen text
    /// was doing. Scripts three distinct texts before the text repeats,
    /// so a correct text-comparing settle wait must keep polling through
    /// all three changes and stop only on the first repeat -- proving the
    /// wait watches text, not revision.
    #[cfg(unix)]
    #[tokio::test]
    async fn sendinput_keeps_polling_while_text_changes_despite_flat_revision() {
        let (elapsed, outcome) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("herdr.sock");
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            let client = SocketHerdr::with_settle_durations_for_test(
                path.clone(),
                TEST_SETTLE_MIN_QUIET,
                TEST_SETTLE_POLL_INTERVAL,
                TEST_SETTLE_MAX_WAIT,
            );

            // "a", "ab", "abc" all differ; "abc" then repeats -- the
            // settle wait must poll through the first three (four reads
            // total) and stop on the fourth, which is the first repeat. A
            // fifth pane.read arriving means stop-on-repeat failed to
            // fire, so the closure itself panics rather than covering for
            // it with a fallback text.
            let texts = ["a", "ab", "abc", "abc"];
            let mut calls = 0usize;
            let server = tokio::spawn(run_settle_mock_server(listener, move || {
                let text = match texts.get(calls) {
                    Some(text) => *text,
                    None => panic!(
                        "wait_for_pane_to_settle must stop polling on the first \
                         repeated text -- a 5th pane.read arrived after only {} \
                         scripted reads",
                        texts.len()
                    ),
                };
                calls += 1;
                text.to_string()
            }));

            let started = tokio::time::Instant::now();
            client.send_input("w1:p1", "hello", true).await.unwrap();
            let elapsed = started.elapsed();
            let outcome = server.await.unwrap();
            (elapsed, outcome)
        })
        .await
        .expect("send_input must not hang");

        // Stopping on the first repeated text -- not the max-wait cap --
        // is what must end this wait; if the cap were doing the work
        // instead, elapsed would sit near TEST_SETTLE_MAX_WAIT rather than
        // well under it.
        assert!(
            elapsed < TEST_SETTLE_MAX_WAIT,
            "the settle wait must stop on the first repeated text, not the cap -- \
             elapsed {elapsed:?} is not well under the {:?} cap",
            TEST_SETTLE_MAX_WAIT
        );

        let reads: Vec<&(Value, tokio::time::Instant)> = outcome
            .iter()
            .filter(|(value, _)| value["method"] == "pane.read")
            .collect();
        assert_eq!(
            reads.len(),
            4,
            "the settle wait must poll exactly 4 times -- a, ab, abc, then the \
             repeated abc that ends it -- but saw {} pane.read call(s): {:?}",
            reads.len(),
            outcome.iter().map(|(value, _)| value).collect::<Vec<_>>()
        );

        let text_request = outcome
            .iter()
            .find(|(value, _)| {
                value["method"] == "pane.send_input" && value["params"].get("text").is_some()
            })
            .expect("at least the text request");
        let first_read = reads.first().expect("at least one pane.read");
        let quiet_before_first_read = first_read.1.saturating_duration_since(text_request.1);
        assert!(
            quiet_before_first_read >= TEST_SETTLE_MIN_QUIET,
            "the first pane.read must not arrive before the min-quiet window has \
             elapsed since the text write -- only {quiet_before_first_read:?} \
             elapsed, want at least {:?}",
            TEST_SETTLE_MIN_QUIET
        );

        let last = outcome.last().expect("at least the enter request");
        assert_eq!(last.0["method"], "pane.send_input");
        assert_eq!(
            last.0["params"]["keys"],
            json!(["enter"]),
            "the settle wait must still send the Enter once the text repeats: {:?}",
            outcome.iter().map(|(value, _)| value).collect::<Vec<_>>()
        );
    }

    #[test]
    fn parse_response_rejects_bad_shape() {
        assert!(matches!(
            parse_response(b"{}"),
            Err(HerdrError::Malformed(_))
        ));
    }

    #[test]
    fn default_socket_path_ends_correctly() {
        #[cfg(windows)]
        assert!(default_socket_path().unwrap().ends_with(
            Path::new("AppData")
                .join("Roaming")
                .join("herdr")
                .join("herdr.sock")
        ));
        #[cfg(not(windows))]
        assert!(default_socket_path()
            .unwrap()
            .ends_with(".config/herdr/herdr.sock"));
    }

    #[test]
    fn resolver_keeps_default_and_builds_named_session_paths() {
        assert_eq!(
            resolve_socket_path("", "default").unwrap(),
            default_socket_path().unwrap()
        );
        #[cfg(windows)]
        assert!(resolve_socket_path("", "team-1").unwrap().ends_with(
            Path::new("AppData")
                .join("Roaming")
                .join("herdr")
                .join("sessions")
                .join("team-1")
                .join("herdr.sock")
        ));
        #[cfg(not(windows))]
        assert!(resolve_socket_path("", "team-1")
            .unwrap()
            .ends_with(".config/herdr/sessions/team-1/herdr.sock"));
    }

    #[test]
    fn resolver_prefers_explicit_override_and_rejects_unsafe_sessions() {
        assert_eq!(
            resolve_socket_path("/custom/herdr.sock", "team").unwrap(),
            PathBuf::from("/custom/herdr.sock")
        );
        assert!(resolve_socket_path("", "../other").is_err());
        assert!(resolve_socket_path("", "name/other").is_err());
    }

    #[test]
    fn injected_config_dir_resolves_without_home() {
        let config_dir = PathBuf::from("C:/Users/operator/AppData/Roaming/herdr");
        assert_eq!(
            resolve_socket_path_from_config_dir("", "default", Ok(config_dir.clone())).unwrap(),
            config_dir.join("herdr.sock")
        );
        assert_eq!(
            resolve_socket_path_from_config_dir("", "team-1", Ok(config_dir.clone())).unwrap(),
            config_dir.join("sessions/team-1/herdr.sock")
        );
    }

    #[test]
    fn unavailable_config_dir_is_a_controlled_error() {
        let error = resolve_socket_path_from_config_dir(
            "",
            "default",
            Err(HerdrError::Unavailable(
                "native Windows roaming application data is unavailable".into(),
            )),
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("native Windows roaming application data is unavailable"));
    }

    /// The injected seam replacing herdr-go's `crate::config::native_roaming_app_data()`
    /// edge (herdr-go/src/herdr/socket.rs:35) -- proves the Windows config dir
    /// resolves purely from an injected closure, with no reach into waggledance's
    /// own config module.
    #[cfg(windows)]
    #[test]
    fn injected_roaming_app_data_resolves_config_dir() {
        let root = PathBuf::from(r"C:\Users\op\AppData\Roaming");
        let dir = herdr_config_dir_with(|| Ok(root.clone())).unwrap();
        assert_eq!(dir, root.join("herdr"));
    }

    #[cfg(windows)]
    #[test]
    fn injected_roaming_app_data_failure_is_unavailable() {
        let err = herdr_config_dir_with(|| {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no roaming app data",
            ))
        })
        .unwrap_err();
        assert!(matches!(err, HerdrError::Unavailable(_)));
    }

    #[cfg(windows)]
    #[test]
    fn windows_endpoint_name_matches_generic_namespaced_mapping() {
        let path = Path::new(r"C:\Users\operator\AppData\Roaming\herdr\herdr.sock");
        let direct = path.as_os_str().to_ns_name::<GenericNamespaced>().unwrap();
        let gateway = windows_endpoint_name(path).unwrap();
        assert_eq!(format!("{direct:?}"), format!("{gateway:?}"));
    }
}
