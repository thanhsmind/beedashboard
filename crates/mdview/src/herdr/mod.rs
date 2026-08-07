//! herdr port — mdview is a **client of the herdr server** (peer to the
//! TUI), talking `herdr.sock`'s JSON request/response API (DISCOVERY 2026-07-18).
//!
//! Ported from herdr-go (`docs/history/agent-terminal/CONTEXT.md` D1) into
//! the `mdview` binary crate, not `mdview-core`: `mdview-core` is enforced
//! async-runtime-free (`bee::tests::no_web_framework_dependency_declared`
//! asserts no `tokio`/`axum`/`hyper` in its `Cargo.toml`), and this client
//! needs `tokio` for the socket I/O. `mdview` already depends on both tokio
//! and axum, so it carries the extra weight this port needs no axum for.
//! `pane_scroller.rs` ports herdr-go's alt-screen scrollback nudger; it has
//! no route-level consumer yet (see its own module doc).
//!
//! The surface is request/response only (no live stream): snapshot the runtime,
//! read a pane's screen (polled), and send input as a reply. One trait, two
//! implementations — [`socket::SocketHerdr`] over the real socket and
//! [`fake::FakeHerdr`] for tests.

pub mod fake;
pub mod pane_scroller;
pub mod socket;
pub mod wire;

use async_trait::async_trait;

pub use wire::{Agent, AgentStatus, ProtocolInfo, ScreenRead, Snapshot, HERDR_PROTOCOL};

#[derive(Debug, thiserror::Error)]
pub enum HerdrError {
    #[error("herdr runtime is unavailable: {0}")]
    Unavailable(String),
    #[error("protocol mismatch: gateway pins {expected}, server reports {actual}")]
    ProtocolMismatch { expected: u32, actual: u32 },
    #[error("herdr request failed: {0}")]
    Request(String),
    #[error("malformed herdr response: {0}")]
    Malformed(String),
    #[error("no such pane: {0}")]
    NoSuchPane(String),
    #[error("agent name already in use: {name} ({message})")]
    AgentNameTaken { name: String, message: String },
    #[error("workspace not found: {workspace_id} ({message})")]
    WorkspaceNotFound {
        workspace_id: String,
        message: String,
    },
    #[error("invalid agent argv: {0}")]
    InvalidAgentArgv(String),
    #[error("herdr refused the request ({code}): {message}")]
    Remote { code: String, message: String },
}

pub type Result<T> = std::result::Result<T, HerdrError>;

/// Which of herdr's own `pane.read` sources to request (matches herdr's own
/// `source` vocabulary, not a gateway invention). `Visible` is the current
/// on-screen rows; `Recent` is herdr's own scrollback buffer, hard-capped at
/// 1000 lines server-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadSource {
    Visible,
    Recent,
}

impl ReadSource {
    /// The exact wire string herdr's `pane.read` expects for `source`.
    pub fn as_wire(&self) -> &'static str {
        match self {
            ReadSource::Visible => "visible",
            ReadSource::Recent => "recent",
        }
    }
}

/// Result of `tab.create` — the new tab's id and its root pane's id, both
/// opaque and read straight off the response, never constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabCreated {
    pub tab_id: String,
    pub pane_id: String,
}

/// Result of `agent.start` — the new agent's pane/tab ids, plus the name
/// that actually succeeded (auto-generated, and only ever different from an
/// earlier attempt after that attempt collided).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStarted {
    pub tab_id: String,
    pub pane_id: String,
    pub name: String,
}

/// Auto-generate an agent name — the caller never types one.
fn generate_agent_name() -> String {
    format!("mobile-agent-{:06x}", rand::random::<u32>() & 0xff_ffff)
}

/// Owns the collision retry so callers never see `AgentNameTaken` themselves:
/// generate a name, try it, and on `AgentNameTaken` regenerate and try again.
/// Bounded at 5 attempts — a sixth consecutive collision means the name
/// generator itself is producing bad names, and looping harder would only
/// hide that, so the bound surfaces its own distinguishable `AgentNameTaken`
/// instead of silently giving up or looping forever.
///
/// Generic over both the name source and the "try once" call, so the retry
/// logic itself is testable against a deterministic name sequence,
/// independent of the real (random) generator `agent_start` uses in
/// production — the same pure-seam idea as `tab_create_params` /
/// `attach_workspace_id` in `socket.rs`.
async fn retry_on_name_collision<G, F, Fut>(
    mut generate_name: G,
    mut try_once: F,
) -> Result<AgentStarted>
where
    G: FnMut() -> String,
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<AgentStarted>>,
{
    const MAX_ATTEMPTS: u32 = 5;
    let mut last_collision: Option<(String, String)> = None;
    for _ in 0..MAX_ATTEMPTS {
        let name = generate_name();
        match try_once(name.clone()).await {
            Ok(started) => return Ok(started),
            Err(HerdrError::AgentNameTaken { name, message }) => {
                last_collision = Some((name, message));
            }
            Err(other) => return Err(other),
        }
    }
    let (name, message) = last_collision.unwrap_or_default();
    Err(HerdrError::AgentNameTaken {
        name,
        message: format!(
            "gave up after {MAX_ATTEMPTS} consecutive name collisions ({message}); the name generator may be broken"
        ),
    })
}

/// Everything mdview needs from herdr — all request/response.
#[async_trait]
pub trait Herdr: Send + Sync {
    /// Snapshot the server's runtime (the flat agent list).
    async fn snapshot(&self) -> Result<Snapshot>;

    /// Health + protocol handshake; a mismatch is a typed error.
    async fn ping(&self) -> Result<ProtocolInfo>;

    /// Read one pane's rendered screen (polled for observation). `source`
    /// selects herdr's own `visible` (current on-screen rows) or `recent`
    /// (scrollback) read; `lines` is honored only for `Recent` (herdr ignores
    /// it for `Visible`) and is capped at herdr's own 1000-line server-side
    /// limit.
    async fn read_pane(
        &self,
        pane_id: &str,
        source: ReadSource,
        lines: usize,
    ) -> Result<ScreenRead>;

    /// Send a reply into a pane. `text` is typed in; `submit` then sends Enter
    /// (handles herdr's send≠submit: text alone does not submit).
    async fn send_input(&self, pane_id: &str, text: &str, submit: bool) -> Result<()>;

    /// Send raw bytes into a pane via herdr's `pane.send_text` channel — no
    /// bracketed-paste wrapping, no named-key translation, exactly the bytes
    /// given. Used to replay a VT escape sequence an alt-screen agent's own
    /// process interprets as a scroll gesture; never `send_keys`/`send_input`
    /// for this purpose.
    async fn send_text(&self, pane_id: &str, bytes: &str) -> Result<()>;

    /// Send raw key presses to a pane — e.g. arrow keys to drive a TUI option
    /// menu, or Enter/Escape/Tab. Key names are herdr's (`up`, `down`, `enter`,
    /// `escape`, `tab`, …).
    async fn send_keys(&self, pane_id: &str, keys: &[String]) -> Result<()>;

    /// Create a plain shell tab in `workspace_id`, never stealing the
    /// desktop's focus (`focus: false`). Returns the new tab's id and its
    /// root pane's id.
    ///
    /// `cwd` is optional: `Some(path)` seeds that exact directory; `None`
    /// omits the key and lets herdr resolve the **workspace's own anchor**
    /// (its focused pane's folder), which is exactly what the desktop does.
    /// Omitting it here is safe — see the asymmetry warning on
    /// [`Herdr::agent_start`].
    async fn tab_create(&self, workspace_id: &str, cwd: Option<&str>) -> Result<TabCreated>;

    /// Start a named agent in `workspace_id`, never stealing the desktop's
    /// focus (`focus: false`). No `tab_id`/`split` is sent — upstream's
    /// default placement (split Right off the workspace's active tab) is
    /// accepted as-is. The name is auto-generated and a collision retried
    /// transparently (see `retry_on_name_collision`): callers never see
    /// `AgentNameTaken` themselves. Returns the new pane's and tab's ids plus
    /// the name that actually succeeded.
    ///
    /// **`cwd` is asymmetric with [`Herdr::tab_create`] — do not assume they
    /// fall back alike.** `Some(path)` seeds that exact directory. `None`
    /// omits the key, but herdr does **not** resolve the workspace anchor for
    /// `agent.start`: it falls back to the **herdr process's own current
    /// directory**, an arbitrary folder unrelated to the destination.
    /// Starting an agent there is the silent wrong-repo start a caller must
    /// refuse rather than pass `None` here — unlike `tab_create`, where
    /// omitting is safe.
    async fn agent_start(
        &self,
        workspace_id: &str,
        cwd: Option<&str>,
        argv: &[String],
    ) -> Result<AgentStarted>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic "already used" set, standing in for a real snapshot's
    /// agents[] just for this pure retry logic -- proves the loop itself
    /// (regenerate on collision, stop on success or on the bound) without
    /// depending on FakeHerdr's randomness or state.
    async fn try_against(taken: &[&str], name: String) -> Result<AgentStarted> {
        if taken.contains(&name.as_str()) {
            Err(HerdrError::AgentNameTaken {
                name: name.clone(),
                message: format!("{name} is already used"),
            })
        } else {
            Ok(AgentStarted {
                tab_id: "w1:t1".into(),
                pane_id: "w1:p9".into(),
                name,
            })
        }
    }

    #[tokio::test]
    async fn agentstart_retries_once_then_succeeds() {
        // The first generated name collides, the second does not -- the
        // caller must end up with the second name and never see the
        // collision itself.
        let names = ["taken-1".to_string(), "free-1".to_string()];
        let mut next = names.into_iter();
        let taken = ["taken-1"];
        let result = retry_on_name_collision(
            || next.next().expect("only 2 attempts expected"),
            |name| try_against(&taken, name),
        )
        .await
        .unwrap();
        assert_eq!(result.name, "free-1");
    }

    #[tokio::test]
    async fn agentstart_gives_up_after_five_collisions() {
        // All 5 generated names collide -- the bound must stop the loop
        // with a terminal, distinguishable error rather than looping
        // forever or silently reporting success.
        let names = [
            "taken-1".to_string(),
            "taken-2".to_string(),
            "taken-3".to_string(),
            "taken-4".to_string(),
            "taken-5".to_string(),
        ];
        let mut next = names.clone().into_iter();
        let taken = names.iter().map(String::as_str).collect::<Vec<_>>();
        let err = retry_on_name_collision(
            || next.next().expect("exactly 5 attempts expected"),
            |name| try_against(&taken, name),
        )
        .await
        .unwrap_err();
        match err {
            HerdrError::AgentNameTaken { name, message } => {
                assert_eq!(name, "taken-5", "carries the last attempted name");
                assert!(
                    message.contains("gave up after 5"),
                    "message must distinguish a bound-exhausted failure from a single collision: {message}"
                );
            }
            other => panic!("expected AgentNameTaken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn agentstart_succeeds_immediately_with_no_collision() {
        let mut next = std::iter::once("free-0".to_string());
        let err_free: [&str; 0] = [];
        let result =
            retry_on_name_collision(|| next.next().unwrap(), |name| try_against(&err_free, name))
                .await
                .unwrap();
        assert_eq!(result.name, "free-0");
    }
}
