//! herdr wire types — the real herdr 0.7.4 socket shapes, verified live
//! (`.bee/spikes/herdr-socket-observe/`, DISCOVERY 2026-07-18). The socket speaks
//! newline-delimited JSON, one request→response per connection.

use serde::{Deserialize, Deserializer, Serialize};

/// Pinned wire protocol number — exact-match (bumps per herdr release).
pub const HERDR_PROTOCOL: u32 = 16;

/// A socket request: `{ "id", "method", "params" }`, newline-terminated.
#[derive(Debug, Serialize)]
pub struct Request<'a> {
    pub id: String,
    pub method: &'a str,
    pub params: serde_json::Value,
}

/// Agent readiness as herdr reports it. `Unknown` catches any value herdr adds
/// later (`#[serde(other)]`), so a new status never breaks the switcher.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Working,
    Blocked,
    Done,
    Idle,
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Working => "working",
            AgentStatus::Blocked => "blocked",
            AgentStatus::Done => "done",
            AgentStatus::Idle => "idle",
            AgentStatus::Unknown => "unknown",
        }
    }
}

/// One agent in the flat snapshot (herdr's `session.snapshot` returns a flat
/// `agents[]`, not a nested tree). `pane_id` is opaque (`w3:p6`), read fresh.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Agent {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    /// Agent kind (herdr field `agent`, e.g. "claude", "codex").
    #[serde(rename = "agent")]
    pub kind: String,
    /// The agent's own name (herdr field `name`) -- distinct from `kind`:
    /// this is the identifier `agent.start` was given (auto-generated), not
    /// the detected tool type. `Agent`'s `Deserialize` is hand-rolled below,
    /// not derived, so the default-when-absent behavior lives on
    /// `RawAgent.name` there, not on this attribute.
    pub name: String,
    #[serde(rename = "agent_status")]
    pub status: AgentStatus,
    /// Human-readable terminal title (herdr `terminal_title_stripped`).
    #[serde(rename = "terminal_title_stripped", default)]
    pub title: String,
}

impl<'de> Deserialize<'de> for Agent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawAgent {
            pane_id: String,
            workspace_id: String,
            tab_id: String,
            #[serde(default)]
            agent: String,
            #[serde(default)]
            name: String,
            #[serde(rename = "agent_status")]
            status: AgentStatus,
            #[serde(rename = "terminal_title_stripped", default)]
            title: String,
        }

        let raw = RawAgent::deserialize(deserializer)?;
        // The Windows fallback (raw.agent empty -> raw.name) is unchanged;
        // `name` is additionally kept in full as its own field below, so a
        // clone is taken here rather than moving raw.name into `kind`.
        let kind = if raw.agent.is_empty() {
            raw.name.clone()
        } else {
            raw.agent
        };
        Ok(Agent {
            pane_id: raw.pane_id,
            workspace_id: raw.workspace_id,
            tab_id: raw.tab_id,
            kind,
            name: raw.name,
            status: raw.status,
            title: raw.title,
        })
    }
}

/// One workspace entry from `session.snapshot.workspaces[]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Workspace {
    pub workspace_id: String,
    pub label: String,
    pub agent_status: AgentStatus,
    /// The workspace's **own** active tab — independent of global focus, and the
    /// first hop of the anchor-pane join.
    #[serde(default)]
    pub active_tab_id: Option<String>,
}

/// One tab entry from `session.snapshot.tabs[]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tab {
    pub tab_id: String,
    pub label: String,
}

/// One pane entry from `session.snapshot.panes[]`.
///
/// Not a substitute for, nor substitutable by, `agents[]`: a plain shell pane
/// carries a folder but never appears in `agents[]`, and it is frequently the
/// anchor of its workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Pane {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    /// The pane process's own directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The foreground child's directory (unix only — `None` elsewhere).
    /// `foreground_cwd ?? cwd` is the folder a create call is seeded with.
    #[serde(default)]
    pub foreground_cwd: Option<String>,
}

/// One entry of `session.snapshot.layouts[]` — herdr emits one per tab of
/// **every** workspace, not just the focused one.
///
/// Deliberately partial: only the fields the anchor join needs. The layout's
/// own `splits[]`/`area`/geometry `panes[]` are not modelled — the split
/// ratio is a float, which would break `Snapshot`'s `Eq` derive. Note that
/// the layout's inner `panes[]` is pane *geometry*, a different thing from
/// the snapshot's top-level `panes[]` of [`Pane`] that carries the folders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PaneLayout {
    pub workspace_id: String,
    pub tab_id: String,
    /// herdr drops a layout entry whole when its focused pane has no public id,
    /// so this can legitimately be absent.
    #[serde(default)]
    pub focused_pane_id: Option<String>,
}

/// The runtime snapshot: everything herdr currently has alive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub agents: Vec<Agent>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub layouts: Vec<PaneLayout>,
    /// Global focus — describes only the one globally active workspace, so it is
    /// usable for preselecting a default destination and **never** for resolving
    /// a workspace's anchor pane.
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub focused_tab_id: Option<String>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
}

/// The anchor pane's folder for a workspace, plus whether that path is the
/// pane's live directory (`foreground_cwd`) or its process start directory
/// (the `cwd` fallback). `Snapshot::anchor_cwd_for_workspace` is this
/// struct's single caller and discards the flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorCwd {
    pub path: String,
    /// `true` only when `path` came from `foreground_cwd`.
    pub live: bool,
}

impl Snapshot {
    /// Human display path for one agent's pane.
    pub fn display_for(agent: &Agent) -> String {
        let t = if agent.title.is_empty() {
            agent.kind.clone()
        } else {
            agent.title.clone()
        };
        format!("{} · {}", agent.kind, t)
    }

    /// Resolve an agent's workspace label by joining on `workspace_id`.
    /// Empty string on a join miss — never panics.
    pub fn workspace_label_for(&self, agent: &Agent) -> String {
        self.workspaces
            .iter()
            .find(|w| w.workspace_id == agent.workspace_id)
            .map(|w| w.label.clone())
            .unwrap_or_default()
    }

    /// Resolve a workspace label directly by id, for rows with no `Agent`
    /// (a shell row's pane carries `workspace_id` but no agent record).
    /// Empty string on a join miss — never panics.
    pub fn workspace_label_for_id(&self, workspace_id: &str) -> String {
        self.workspaces
            .iter()
            .find(|w| w.workspace_id == workspace_id)
            .map(|w| w.label.clone())
            .unwrap_or_default()
    }

    /// Resolve an agent's tab label by joining on `tab_id`. Empty string on a
    /// join miss — never panics.
    pub fn tab_label_for(&self, agent: &Agent) -> String {
        self.tabs
            .iter()
            .find(|t| t.tab_id == agent.tab_id)
            .map(|t| t.label.clone())
            .unwrap_or_default()
    }

    /// Resolve a tab label directly by id, for rows with no `Agent` (a shell
    /// row's pane carries `tab_id` but no agent record). Empty string on a
    /// join miss — never panics.
    pub fn tab_label_for_id(&self, tab_id: &str) -> String {
        self.tabs
            .iter()
            .find(|t| t.tab_id == tab_id)
            .map(|t| t.label.clone())
            .unwrap_or_default()
    }

    /// Resolve an agent's workspace status rollup by joining on `workspace_id`.
    /// Falls back to `AgentStatus::Unknown` on a join miss — never panics.
    pub fn workspace_status_for(&self, agent: &Agent) -> AgentStatus {
        self.workspaces
            .iter()
            .find(|w| w.workspace_id == agent.workspace_id)
            .map(|w| w.agent_status)
            .unwrap_or(AgentStatus::Unknown)
    }

    /// Resolve the anchor pane's folder and its provenance for **any**
    /// workspace, reproducing herdr's own `focused_pane_cwd_in_workspace`.
    ///
    /// The join: `workspace_id` -> that workspace's own `active_tab_id` -> the
    /// `layouts[]` entry whose `workspace_id` **and** `tab_id` both match ->
    /// its `focused_pane_id` -> that pane in the snapshot's top-level
    /// `panes[]` -> `foreground_cwd ?? cwd`, with `live` true only for the
    /// `foreground_cwd` branch.
    ///
    /// Deliberately does **not** use the snapshot's top-level
    /// `focused_workspace_id`/`focused_tab_id`/`focused_pane_id` or a pane's
    /// own `focused` flag — those are global and describe only the one
    /// active workspace, which is wrong for every other workspace. A miss at
    /// any hop degrades to `None`, never a panic and never a substitute
    /// pane, mirroring the join-miss fallbacks above.
    pub fn anchor_for_workspace(&self, workspace_id: &str) -> Option<AnchorCwd> {
        let workspace = self
            .workspaces
            .iter()
            .find(|w| w.workspace_id == workspace_id)?;
        let active_tab_id = workspace.active_tab_id.as_deref()?;
        let layout = self
            .layouts
            .iter()
            .find(|l| l.workspace_id == workspace_id && l.tab_id == active_tab_id)?;
        let focused_pane_id = layout.focused_pane_id.as_deref()?;
        let pane = self.panes.iter().find(|p| p.pane_id == focused_pane_id)?;
        if let Some(path) = pane.foreground_cwd.clone() {
            Some(AnchorCwd { path, live: true })
        } else {
            pane.cwd.clone().map(|path| AnchorCwd { path, live: false })
        }
    }

    /// Resolve the anchor pane's folder for **any** workspace. Delegates to
    /// [`Snapshot::anchor_for_workspace`], discarding provenance.
    pub fn anchor_cwd_for_workspace(&self, workspace_id: &str) -> Option<String> {
        self.anchor_for_workspace(workspace_id).map(|a| a.path)
    }
}

/// A polled screen read of one pane (`pane.read`). `text` is the rendered
/// screen (ANSI or plain); `revision` bumps when the pane content changes, so a
/// poller can skip re-rendering an unchanged screen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ScreenRead {
    pub text: String,
    pub revision: u64,
}

/// Health/handshake info (`ping` → pong). Pinned exact-match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolInfo {
    pub protocol: u32,
    pub server_version: String,
}

impl ProtocolInfo {
    pub fn is_compatible(&self) -> bool {
        self.protocol == HERDR_PROTOCOL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_status_parses_all_including_unknown() {
        let p = |s: &str| serde_json::from_str::<AgentStatus>(&format!("\"{s}\"")).unwrap();
        assert_eq!(p("working"), AgentStatus::Working);
        assert_eq!(p("done"), AgentStatus::Done);
        assert_eq!(p("idle"), AgentStatus::Idle);
        // an unexpected value falls back to Unknown, not an error
        assert_eq!(p("something_new"), AgentStatus::Unknown);
    }

    #[test]
    fn flat_snapshot_parses_real_shape() {
        // Shape captured live from `session.snapshot`.
        let json = r#"{
          "agents": [
            {"agent":"claude","agent_status":"idle","pane_id":"w3:p6","workspace_id":"w3","tab_id":"w3:t6","terminal_title_stripped":"Kiểm tra plan"}
          ]
        }"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.agents.len(), 1);
        let a = &snap.agents[0];
        assert_eq!(a.pane_id, "w3:p6");
        assert_eq!(a.kind, "claude");
        assert_eq!(a.status, AgentStatus::Idle);
        assert_eq!(a.title, "Kiểm tra plan");
    }

    #[test]
    fn flat_snapshot_parses_windows_started_agent_name_fallback() {
        // Windows `herdr agent start <name> -- cmd.exe ...` can report the
        // created pane with `name` and no `agent` field.
        let json = r#"{
          "agents": [
            {
              "agent_status":"unknown",
              "cwd":"D:\\a\\herdr-go\\herdr-go\\",
              "focused":true,
              "name":"gateway-smoke",
              "pane_id":"w1:p1",
              "revision":2,
              "tab_id":"w1:t1",
              "terminal_id":"term_656f050b6721a1",
              "terminal_title":"Administrator: C:\\Windows\\system32\\cmd.exe",
              "terminal_title_stripped":"Administrator: C:\\Windows\\system32\\cmd.exe",
              "workspace_id":"w1"
            }
          ]
        }"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.agents.len(), 1);
        let a = &snap.agents[0];
        assert_eq!(a.pane_id, "w1:p1");
        assert_eq!(a.kind, "gateway-smoke");
        assert_eq!(a.status, AgentStatus::Unknown);
        assert_eq!(a.title, "Administrator: C:\\Windows\\system32\\cmd.exe");
        // The Windows kind-fallback is unchanged by this field's addition --
        // `name` is populated in full alongside it, not instead of it.
        assert_eq!(a.name, "gateway-smoke");
    }

    #[test]
    fn agentstart_wire_agent_parses_name_field() {
        let json = r#"{
          "agents": [
            {"agent":"claude","agent_status":"idle","pane_id":"w3:p6","workspace_id":"w3","tab_id":"w3:t6","name":"mobile-agent-1"}
          ]
        }"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.agents[0].name, "mobile-agent-1");
        // `agent` (kind) is present here too, so the Windows fallback must
        // not have kicked in and overwritten it.
        assert_eq!(snap.agents[0].kind, "claude");
    }

    #[test]
    fn agentstart_wire_agent_name_defaults_empty_when_absent() {
        // Every fixture predating this field has no "name" key at all.
        let json = r#"{
          "agents": [
            {"agent":"claude","agent_status":"idle","pane_id":"w3:p6","workspace_id":"w3","tab_id":"w3:t6"}
          ]
        }"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.agents[0].name, "");
    }

    #[test]
    fn snapshot_parses_workspaces_and_tabs_and_resolves_labels() {
        // Shape captured live from `session.snapshot`, including workspaces[]/tabs[].
        let json = r#"{
          "agents": [
            {"agent":"claude","agent_status":"idle","pane_id":"w3:p6","workspace_id":"w3","tab_id":"w3:t6","terminal_title_stripped":"Kiểm tra plan"}
          ],
          "workspaces": [
            {"workspace_id":"w3","label":"herdr-gateway","agent_status":"working"}
          ],
          "tabs": [
            {"tab_id":"w3:t6","label":"chat"}
          ]
        }"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        let a = &snap.agents[0];
        assert_eq!(snap.workspace_label_for(a), "herdr-gateway");
        assert_eq!(snap.tab_label_for(a), "chat");
        assert_eq!(snap.workspace_status_for(a), AgentStatus::Working);
    }

    #[test]
    fn workspace_status_falls_back_to_unknown_on_join_miss() {
        let snap = Snapshot {
            agents: vec![],
            workspaces: vec![],
            tabs: vec![],
            ..Snapshot::default()
        };
        let a = Agent {
            pane_id: "w9:p1".into(),
            workspace_id: "w9".into(),
            tab_id: "w9:t1".into(),
            kind: "claude".into(),
            name: String::new(),
            status: AgentStatus::Idle,
            title: String::new(),
        };
        assert_eq!(snap.workspace_status_for(&a), AgentStatus::Unknown);
    }

    #[test]
    fn label_resolution_falls_back_to_empty_string_on_join_miss() {
        let snap = Snapshot {
            agents: vec![],
            workspaces: vec![],
            tabs: vec![],
            ..Snapshot::default()
        };
        let a = Agent {
            pane_id: "w9:p1".into(),
            workspace_id: "w9".into(),
            tab_id: "w9:t1".into(),
            kind: "claude".into(),
            name: String::new(),
            status: AgentStatus::Idle,
            title: String::new(),
        };
        assert_eq!(snap.workspace_label_for(&a), "");
        assert_eq!(snap.tab_label_for(&a), "");
    }

    /// The captured herdr 0.7.4 / protocol 16 envelope, tracked test data. This
    /// is the INNER `snapshot` object, so it deserializes straight into
    /// `Snapshot`; the socket path wraps it (see `socket.rs`).
    const LIVE_SNAPSHOT: &str = include_str!("testdata/live-snapshot.json");
    const EXPECTED_ANCHORS: &str = include_str!("testdata/expected-anchors.json");

    #[test]
    fn envelope_parses_panes_and_layouts() {
        let snap: Snapshot = serde_json::from_str(LIVE_SNAPSHOT).unwrap();

        assert_eq!(snap.workspaces.len(), 5);
        assert_eq!(snap.panes.len(), 8);
        assert_eq!(snap.layouts.len(), 5);

        // Every workspace carries its own active tab (D10 hop 1).
        assert!(snap.workspaces.iter().all(|w| w.active_tab_id.is_some()));
        // Every layout entry names the tab it belongs to and its focused pane.
        assert!(snap.layouts.iter().all(|l| l.focused_pane_id.is_some()));
        // Panes carry folders — the whole reason panes[] is parsed.
        assert!(snap.panes.iter().all(|p| p.cwd.is_some()));
        assert!(snap.panes.iter().any(|p| p.foreground_cwd.is_some()));

        let p = snap.panes.iter().find(|p| p.pane_id == "w3:p6").unwrap();
        assert_eq!(p.workspace_id, "w3");
        assert_eq!(p.tab_id, "w3:t6");
        assert_eq!(
            p.foreground_cwd.as_deref(),
            Some("/home/dev/projects/fgos/fgos-dev")
        );

        // Global focus is parsed, but only ever for preselecting a default.
        assert_eq!(snap.focused_workspace_id.as_deref(), Some("wB"));
        assert_eq!(snap.focused_tab_id.as_deref(), Some("wB:t1"));
        assert_eq!(snap.focused_pane_id.as_deref(), Some("wB:p1"));
    }

    #[test]
    fn envelope_carries_anchor_join_inputs_for_every_workspace() {
        // Not the join itself (that is a later cell) — this asserts the parsed
        // surface actually contains every hop D10 needs, for all five captured
        // workspaces including the four that are not globally focused.
        let snap: Snapshot = serde_json::from_str(LIVE_SNAPSHOT).unwrap();
        let expected: serde_json::Value = serde_json::from_str(EXPECTED_ANCHORS).unwrap();
        let rows = expected["workspaces"].as_array().unwrap();
        assert_eq!(rows.len(), 5);

        for row in rows {
            let ws_id = row["workspace_id"].as_str().unwrap();
            let tab_id = row["active_tab_id"].as_str().unwrap();
            let pane_id = row["anchor_pane_id"].as_str().unwrap();
            let cwd = row["expected_cwd"].as_str().unwrap();

            let ws = snap
                .workspaces
                .iter()
                .find(|w| w.workspace_id == ws_id)
                .unwrap();
            assert_eq!(ws.active_tab_id.as_deref(), Some(tab_id));

            let layout = snap
                .layouts
                .iter()
                .find(|l| l.workspace_id == ws_id && l.tab_id == tab_id)
                .unwrap();
            assert_eq!(layout.focused_pane_id.as_deref(), Some(pane_id));

            let pane = snap.panes.iter().find(|p| p.pane_id == pane_id).unwrap();
            assert_eq!(
                pane.foreground_cwd.as_deref().or(pane.cwd.as_deref()),
                Some(cwd)
            );

            // agents[] is not a substitute for panes[]: the capture's globally
            // focused workspace is anchored on a plain shell with no agent.
            if row["anchor_is_plain_shell_absent_from_agents"] == serde_json::Value::Bool(true) {
                assert!(!snap.agents.iter().any(|a| a.pane_id == pane_id));
            }
        }
    }

    #[test]
    fn envelope_missing_new_fields_still_parses() {
        // An older/partial server sends no panes[], no layouts[], no
        // active_tab_id and no focused_*_id: the serde layer tolerates it.
        let json = r#"{
          "agents": [
            {"agent":"claude","agent_status":"idle","pane_id":"w3:p6","workspace_id":"w3","tab_id":"w3:t6","terminal_title_stripped":"t"}
          ],
          "workspaces": [
            {"workspace_id":"w3","label":"herdr-gateway","agent_status":"working"}
          ],
          "tabs": [{"tab_id":"w3:t6","label":"chat"}]
        }"#;
        let snap: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.agents.len(), 1);
        assert!(snap.panes.is_empty());
        assert!(snap.layouts.is_empty());
        assert_eq!(snap.workspaces[0].active_tab_id, None);
        assert_eq!(snap.focused_workspace_id, None);
        assert_eq!(snap.focused_tab_id, None);
        assert_eq!(snap.focused_pane_id, None);

        // A pane with neither folder is a tolerated shape too.
        let pane: Pane =
            serde_json::from_str(r#"{"pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1"}"#)
                .unwrap();
        assert_eq!(pane.cwd, None);
        assert_eq!(pane.foreground_cwd, None);

        // As is a layout entry whose focused pane has no public id.
        let layout: PaneLayout =
            serde_json::from_str(r#"{"workspace_id":"w1","tab_id":"w1:t1"}"#).unwrap();
        assert_eq!(layout.focused_pane_id, None);
    }

    // --- D10 anchor join (cell new-shell-new-agent-2) ---------------------

    fn test_workspace(id: &str, active_tab_id: Option<&str>) -> Workspace {
        Workspace {
            workspace_id: id.into(),
            label: String::new(),
            agent_status: AgentStatus::Unknown,
            active_tab_id: active_tab_id.map(|s| s.into()),
        }
    }

    fn test_layout(workspace_id: &str, tab_id: &str, focused_pane_id: Option<&str>) -> PaneLayout {
        PaneLayout {
            workspace_id: workspace_id.into(),
            tab_id: tab_id.into(),
            focused_pane_id: focused_pane_id.map(|s| s.into()),
        }
    }

    fn test_pane(
        pane_id: &str,
        workspace_id: &str,
        tab_id: &str,
        cwd: Option<&str>,
        foreground_cwd: Option<&str>,
    ) -> Pane {
        Pane {
            pane_id: pane_id.into(),
            workspace_id: workspace_id.into(),
            tab_id: tab_id.into(),
            cwd: cwd.map(|s| s.into()),
            foreground_cwd: foreground_cwd.map(|s| s.into()),
        }
    }

    /// Two independent workspaces, neither of which is the globally focused
    /// one from the snapshot's point of view (`focused_workspace_id` names a
    /// third, absent id) -- proves the join does not secretly depend on
    /// global focus.
    fn two_workspace_snapshot() -> Snapshot {
        Snapshot {
            workspaces: vec![
                test_workspace("w1", Some("w1:t1")),
                test_workspace("w2", Some("w2:t1")),
            ],
            layouts: vec![
                test_layout("w1", "w1:t1", Some("w1:p1")),
                test_layout("w2", "w2:t1", Some("w2:p1")),
            ],
            panes: vec![
                test_pane("w1:p1", "w1", "w1:t1", Some("/one"), None),
                test_pane("w2:p1", "w2", "w2:t1", Some("/two"), None),
            ],
            focused_workspace_id: Some("w1".into()),
            focused_tab_id: Some("w1:t1".into()),
            focused_pane_id: Some("w1:p1".into()),
            ..Snapshot::default()
        }
    }

    #[test]
    fn anchor_resolves_for_globally_focused_workspace() {
        let snap = two_workspace_snapshot();
        assert_eq!(snap.anchor_cwd_for_workspace("w1").as_deref(), Some("/one"));
    }

    #[test]
    fn anchor_resolves_for_non_focused_workspace() {
        // w2 is not named by any of the snapshot's top-level focused_*_id
        // fields, yet it must still resolve via its own active_tab_id/layout.
        let snap = two_workspace_snapshot();
        assert_eq!(snap.anchor_cwd_for_workspace("w2").as_deref(), Some("/two"));
    }

    #[test]
    fn anchor_prefers_foreground_cwd_over_cwd() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/cwd"), Some("/fg"))],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_cwd_for_workspace("w1").as_deref(), Some("/fg"));
    }

    #[test]
    fn anchor_uses_cwd_when_foreground_cwd_absent() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/cwd"), None)],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_cwd_for_workspace("w1").as_deref(), Some("/cwd"));
    }

    #[test]
    fn anchor_none_when_pane_missing_both_cwds() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", None, None)],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_cwd_for_workspace("w1"), None);
    }

    #[test]
    fn anchor_none_when_layouts_absent() {
        // Older/partial herdr: no layouts[] at all.
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_cwd_for_workspace("w1"), None);
    }

    #[test]
    fn anchor_none_when_active_tab_has_no_layout_entry() {
        // active_tab_id names a tab that no layouts[] entry carries -- the
        // synthesized-id degrade path.
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t9"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/one"), None)],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_cwd_for_workspace("w1"), None);
    }

    #[test]
    fn anchor_ignores_layout_matched_only_by_tab_id() {
        // A layouts[] entry belonging to workspace w2 happens to share w1's
        // tab_id. Matching on tab_id alone would wrongly resolve w1 through
        // it; the join requires workspace_id AND tab_id both to match.
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("shared:t1"))],
            layouts: vec![test_layout("w2", "shared:t1", Some("w2:pX"))],
            panes: vec![test_pane("w2:pX", "w2", "shared:t1", Some("/wrong"), None)],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_cwd_for_workspace("w1"), None);
    }

    #[test]
    fn anchor_none_when_workspace_id_unknown() {
        let snap = Snapshot::default();
        assert_eq!(snap.anchor_cwd_for_workspace("missing"), None);
    }

    #[test]
    fn anchor_resolves_shell_pane_absent_from_agents() {
        // The anchor pane is a plain shell -- it must resolve from panes[]
        // even though it never appears in agents[].
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/shell"), None)],
            agents: vec![],
            ..Snapshot::default()
        };
        assert!(!snap.agents.iter().any(|a| a.pane_id == "w1:p1"));
        assert_eq!(
            snap.anchor_cwd_for_workspace("w1").as_deref(),
            Some("/shell")
        );
    }

    #[test]
    fn anchor_live_envelope_matches_probe() {
        // The tracked capture has 5 workspaces, 4 of which are not globally
        // focused, and the globally focused one anchors on a plain shell.
        // src/herdr/testdata/expected-anchors.json is a live probe's
        // known-good answer for every one of them.
        let snap: Snapshot = serde_json::from_str(LIVE_SNAPSHOT).unwrap();
        let expected: serde_json::Value = serde_json::from_str(EXPECTED_ANCHORS).unwrap();
        let rows = expected["workspaces"].as_array().unwrap();
        assert_eq!(rows.len(), 5);

        for row in rows {
            let ws_id = row["workspace_id"].as_str().unwrap();
            let expected_cwd = row["expected_cwd"].as_str().unwrap();
            assert_eq!(
                snap.anchor_cwd_for_workspace(ws_id).as_deref(),
                Some(expected_cwd),
                "workspace {ws_id} anchor mismatch"
            );
        }
    }

    // --- provenance-carrying anchor accessor (cell web-create-endpoints-1) -

    #[test]
    fn provenance_true_when_foreground_cwd_present() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/cwd"), Some("/fg"))],
            ..Snapshot::default()
        };
        let anchor = snap.anchor_for_workspace("w1").unwrap();
        assert_eq!(anchor.path, "/fg");
        assert!(anchor.live);
    }

    #[test]
    fn provenance_false_when_only_cwd_present() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/cwd"), None)],
            ..Snapshot::default()
        };
        let anchor = snap.anchor_for_workspace("w1").unwrap();
        assert_eq!(anchor.path, "/cwd");
        assert!(!anchor.live);
    }

    #[test]
    fn provenance_none_when_pane_missing_both_cwds() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", None, None)],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_for_workspace("w1"), None);
    }

    #[test]
    fn provenance_none_when_layouts_absent() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_for_workspace("w1"), None);
    }

    #[test]
    fn provenance_none_when_active_tab_has_no_layout_entry() {
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t9"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/one"), None)],
            ..Snapshot::default()
        };
        assert_eq!(snap.anchor_for_workspace("w1"), None);
    }

    #[test]
    fn provenance_none_when_workspace_id_unknown() {
        let snap = Snapshot::default();
        assert_eq!(snap.anchor_for_workspace("missing"), None);
    }

    #[test]
    fn provenance_resolves_for_non_focused_workspace() {
        // w2 is not named by any of the snapshot's top-level focused_*_id
        // fields, yet it must still resolve via its own active_tab_id/layout.
        let snap = two_workspace_snapshot();
        let anchor = snap.anchor_for_workspace("w2").unwrap();
        assert_eq!(anchor.path, "/two");
        assert!(!anchor.live);
    }

    #[test]
    fn provenance_anchor_cwd_for_workspace_delegates() {
        // anchor_cwd_for_workspace must keep returning exactly the provenance
        // accessor's path, unchanged signature and behavior (slice 1's tests
        // assert this independently too).
        let snap = Snapshot {
            workspaces: vec![test_workspace("w1", Some("w1:t1"))],
            layouts: vec![test_layout("w1", "w1:t1", Some("w1:p1"))],
            panes: vec![test_pane("w1:p1", "w1", "w1:t1", Some("/cwd"), Some("/fg"))],
            ..Snapshot::default()
        };
        assert_eq!(
            snap.anchor_cwd_for_workspace("w1"),
            snap.anchor_for_workspace("w1").map(|a| a.path)
        );
    }

    #[test]
    fn protocol_pin_exact() {
        assert!(ProtocolInfo {
            protocol: 16,
            server_version: "0.7.4".into()
        }
        .is_compatible());
        assert!(!ProtocolInfo {
            protocol: 17,
            server_version: "x".into()
        }
        .is_compatible());
    }
}
