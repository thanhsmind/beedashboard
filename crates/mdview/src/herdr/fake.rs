//! In-memory herdr — mirrors the real socket shapes (flat snapshot, `pane.read`
//! screen buffer, `send_input` echo) so the whole app runs and is tested with no
//! live herdr.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::wire::*;
use super::{
    generate_agent_name, retry_on_name_collision, AgentStarted, Herdr, HerdrError, ReadSource,
    Result, TabCreated,
};

/// The two VT escape sequences a `PaneScroller` (herdr-go's alt-screen
/// scrollback nudger, not carried into this crate — no consumer until
/// slice 3) sends via `send_text`. Kept here, not re-exported, purely so
/// this fake's own `escape_pages`/`scrolled` simulation matches the exact
/// bytes a later slice will send, without pulling `pane_scroller.rs` in
/// ahead of its first consumer.
const PAGE_UP: &str = "\x1b[5~";
const RESTORE_BOTTOM: &str = "\x1b[1;5F";

#[derive(Clone)]
pub struct FakeHerdr {
    inner: Arc<Inner>,
}

struct Inner {
    snapshot: Mutex<Snapshot>,
    screens: Mutex<HashMap<String, PaneScreen>>,
    available: std::sync::atomic::AtomicBool,
    next_created_id: std::sync::atomic::AtomicU64, // suffix for created tab/pane ids
    /// `(pane_id, bytes)` log of every `send_text` call, in order -- lets a
    /// test assert exactly when (and whether) a scroller escalated, without
    /// inferring it from timing.
    sent_text: Mutex<Vec<(String, String)>>,
}

/// One pane's fake screen state -- history-aware so a test can construct all
/// three scroll cases (short pane, primary-screen pane, alt-screen pane)
/// without a live herdr.
#[derive(Clone)]
struct PaneScreen {
    /// What `source: visible` returns right now -- or, while `scrolled` is
    /// true, the current `escape_pages` entry is shown instead (see
    /// `read_pane`).
    visible: String,
    /// What `source: recent` returns (sliced by the requested `lines`,
    /// capped at 1000, mirroring herdr's own limit). Identical to `visible`
    /// by default -- no extra native scrollback to give -- diverges only for
    /// a pane seeded via `seed_scroll_pane`.
    recent: String,
    revision: u64,
    /// Successive pages a raw PageUp reveals for an alt-screen pane that
    /// responds to its own internal scroll keybinding -- empty for a pane
    /// that generically ignores the sequence (the harmless no-op case). Each
    /// additional PageUp while already `scrolled` advances `scroll_index`
    /// one page further, clamped at the last one (multi-page scroll-back).
    escape_pages: Vec<String>,
    /// Which `escape_pages` entry is currently showing.
    scroll_index: usize,
    /// Whether a page of `escape_pages` is currently showing (PageUp sent,
    /// Ctrl+End not sent yet).
    scrolled: bool,
    /// How many `Visible` reads while `scrolled` must happen before
    /// `escape_reveal` actually shows -- models a real alt-screen agent's
    /// asynchronous redraw lag (its TUI's re-render is not synchronous with
    /// the byte that triggered it). 0 (default) matches this fake's
    /// long-standing instant-reveal behavior.
    reveal_after_reads: usize,
    /// Counts `Visible` reads since `scrolled` last flipped true; reset on
    /// every scroll/restore transition.
    reads_since_scroll: usize,
    /// How many `Visible` reads after `scrolled` flips false must happen
    /// before the pane genuinely looks restored -- models the same
    /// asynchronous-redraw lag on the exit side (Ctrl+End's bytes land
    /// before the agent's TUI has actually left its own scroll view). 0
    /// (default) matches this fake's long-standing instant-restore behavior.
    restore_after_reads: usize,
    /// Counts `Visible` reads since `scrolled` last flipped false; reset on
    /// every scroll/restore transition.
    reads_since_restore: usize,
    /// Whether `scrolled` has ever flipped true -- gates `restore_after_reads`
    /// so a pane's pristine pre-scroll state (also `scrolled: false`) is
    /// never mistaken for "just exited a scroll view".
    ever_scrolled: bool,
}

impl PaneScreen {
    fn new(text: String) -> Self {
        PaneScreen {
            recent: text.clone(),
            visible: text,
            revision: 1,
            escape_pages: Vec::new(),
            scroll_index: 0,
            scrolled: false,
            reveal_after_reads: 0,
            reads_since_scroll: 0,
            restore_after_reads: 0,
            reads_since_restore: 0,
            ever_scrolled: false,
        }
    }
}

/// The last `lines` lines of `text` (herdr's own `recent` semantics) -- the
/// whole text when it has fewer lines than requested, never a panic on a
/// short buffer.
fn tail_lines(text: &str, lines: usize) -> String {
    if lines == 0 {
        return String::new();
    }
    let all: Vec<&str> = text.split('\n').collect();
    if all.len() <= lines {
        text.to_string()
    } else {
        all[all.len() - lines..].join("\n")
    }
}

impl Default for FakeHerdr {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHerdr {
    /// Seeded with four agents, one per status, each with a starter screen.
    pub fn new() -> Self {
        let agents = vec![
            agent(
                "w1:p1",
                "claude",
                "claude-main",
                AgentStatus::Working,
                "Building the parser",
            ),
            agent(
                "w1:p2",
                "codex",
                "codex-review",
                AgentStatus::Blocked,
                "Waiting for your answer",
            ),
            agent(
                "w2:p3",
                "claude",
                "claude-docs",
                AgentStatus::Done,
                "Finished the refactor",
            ),
            agent("w2:p4", "codex", "codex-idle", AgentStatus::Idle, "Idle"),
        ];
        // panes[] is a superset of agents[]: w2:p5 is a plain shell with a
        // folder and no agent, and it is w2's anchor — the same shape as the
        // live capture, so the seed exercises the real join instead of the
        // easy case where every anchor happens to be an agent.
        //
        // w3:p6 is the shell-only-workspace anchor: w3 has NO agents at all
        // (the case an agents[]-only view cannot see), and its anchor pane is
        // cwd-only (foreground_cwd absent). Both shapes the live client
        // genuinely produces — a live capture proved the cwd-only anchor — but
        // the old seed could not: every seeded pane set foreground_cwd == cwd,
        // and every seeded workspace had an agent.
        let panes = vec![
            pane("w1:p1", "/home/dev/projects/frontend-app"),
            pane("w1:p2", "/home/dev/projects/frontend-app"),
            pane("w2:p3", "/home/dev/projects/docs-site"),
            pane("w2:p4", "/home/dev/projects/docs-site"),
            pane("w2:p5", "/home/dev/projects/docs-site/site"),
            Pane {
                pane_id: "w3:p6".into(),
                workspace_id: "w3".into(),
                tab_id: "w3:t".into(),
                cwd: Some("/home/dev/projects/backend-api".into()),
                foreground_cwd: None,
            },
            // A second shell pane in the same agentless workspace -- proves a
            // workspace with 2+ shells produces one row per pane, not one per
            // workspace.
            pane("w3:p7", "/home/dev/projects/backend-api/scripts"),
        ];
        let mut screens = HashMap::new();
        for a in &agents {
            screens.insert(
                a.pane_id.clone(),
                PaneScreen::new(format!("{} [{}]\n❯ ", a.title, a.status.as_str())),
            );
        }
        // The plain shells have screens too — they are real panes in this fake.
        screens.insert("w2:p5".to_string(), PaneScreen::new("❯ ".to_string()));
        screens.insert("w3:p6".to_string(), PaneScreen::new("❯ ".to_string()));
        screens.insert("w3:p7".to_string(), PaneScreen::new("❯ ".to_string()));
        FakeHerdr {
            inner: Arc::new(Inner {
                snapshot: Mutex::new(Snapshot {
                    agents,
                    workspaces: vec![
                        Workspace {
                            workspace_id: "w1".into(),
                            label: "frontend-app".into(),
                            agent_status: AgentStatus::Working,
                            active_tab_id: Some("w1:t".into()),
                        },
                        Workspace {
                            workspace_id: "w2".into(),
                            label: "docs-site".into(),
                            agent_status: AgentStatus::Done,
                            active_tab_id: Some("w2:t".into()),
                        },
                        // Shell-only workspace: no agents, so its rollup is
                        // Idle (no work in progress -- not Unknown, which is
                        // reserved for a value this app doesn't recognize).
                        Workspace {
                            workspace_id: "w3".into(),
                            label: "backend-api".into(),
                            agent_status: AgentStatus::Idle,
                            active_tab_id: Some("w3:t".into()),
                        },
                    ],
                    tabs: vec![
                        Tab {
                            tab_id: "w1:t".into(),
                            label: "main".into(),
                        },
                        Tab {
                            tab_id: "w2:t".into(),
                            label: "main".into(),
                        },
                        Tab {
                            tab_id: "w3:t".into(),
                            label: "main".into(),
                        },
                    ],
                    panes,
                    layouts: vec![
                        PaneLayout {
                            workspace_id: "w1".into(),
                            tab_id: "w1:t".into(),
                            focused_pane_id: Some("w1:p1".into()),
                        },
                        PaneLayout {
                            workspace_id: "w2".into(),
                            tab_id: "w2:t".into(),
                            focused_pane_id: Some("w2:p5".into()),
                        },
                        PaneLayout {
                            workspace_id: "w3".into(),
                            tab_id: "w3:t".into(),
                            focused_pane_id: Some("w3:p6".into()),
                        },
                    ],
                    // Only w1 is globally focused; w2 still has its own anchor.
                    focused_workspace_id: Some("w1".into()),
                    focused_tab_id: Some("w1:t".into()),
                    focused_pane_id: Some("w1:p1".into()),
                }),
                screens: Mutex::new(screens),
                available: std::sync::atomic::AtomicBool::new(true),
                next_created_id: std::sync::atomic::AtomicU64::new(1),
                sent_text: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Empty runtime (recovery tests).
    pub fn empty() -> Self {
        let f = Self::new();
        {
            let mut s = f.inner.snapshot.try_lock().expect("fresh, uncontended");
            *s = Snapshot::default();
        }
        f
    }

    pub fn set_available(&self, up: bool) {
        self.inner
            .available
            .store(up, std::sync::atomic::Ordering::SeqCst);
    }

    /// Test-only construction seam: seeds (or overwrites) `pane_id`'s screen
    /// with independently controllable `visible`/`recent`/escape-reveal
    /// shapes -- the only way to build the three scroll cases (short,
    /// primary-screen, alt-screen) without a live herdr.
    pub fn seed_scroll_pane(
        &self,
        pane_id: &str,
        visible: &str,
        recent: &str,
        escape_reveal: Option<&str>,
    ) {
        let screen = PaneScreen {
            visible: visible.to_string(),
            recent: recent.to_string(),
            revision: 1,
            escape_pages: escape_reveal
                .map(|s| vec![s.to_string()])
                .unwrap_or_default(),
            scroll_index: 0,
            scrolled: false,
            reveal_after_reads: 0,
            reads_since_scroll: 0,
            restore_after_reads: 0,
            reads_since_restore: 0,
            ever_scrolled: false,
        };
        let mut screens = self.inner.screens.try_lock().expect("fresh, uncontended");
        screens.insert(pane_id.to_string(), screen);
    }

    /// Test-only: append another page reachable by one more PageUp beyond
    /// what `seed_scroll_pane`'s single `escape_reveal` already seeded --
    /// models multi-page scroll-back (repeated PageUp keeps revealing
    /// successively older content).
    pub fn push_escape_page(&self, pane_id: &str, page: &str) {
        let mut screens = self.inner.screens.try_lock().expect("fresh, uncontended");
        if let Some(screen) = screens.get_mut(pane_id) {
            screen.escape_pages.push(page.to_string());
        }
    }

    /// Test-only: make `pane_id`'s already-seeded escape-reveal only become
    /// visible after `reads` additional `Visible` reads while scrolled --
    /// models a real alt-screen agent's asynchronous redraw lag, reproduced
    /// live against Claude Code 2.1.220 / herdr 0.7.4 (an immediate read
    /// right after `send_text(PAGE_UP)` raced ahead of the TUI's re-render
    /// and returned the stale, pre-scroll screen).
    pub fn set_reveal_delay(&self, pane_id: &str, reads: usize) {
        let mut screens = self.inner.screens.try_lock().expect("fresh, uncontended");
        if let Some(screen) = screens.get_mut(pane_id) {
            screen.reveal_after_reads = reads;
        }
    }

    /// Test-only: make `pane_id` keep showing `escape_reveal` for `reads`
    /// additional `Visible` reads after Ctrl+End is sent (`scrolled` flips
    /// false) -- models the same asynchronous-redraw lag on the exit side,
    /// reproduced live: a real keystroke (e.g. a Reply-sheet Send) arriving
    /// while Claude Code was still mid-transition out of its own scroll view
    /// got swallowed as "dismiss scroll view" instead of reaching the
    /// composer, so typed text landed with no Enter.
    pub fn set_restore_delay(&self, pane_id: &str, reads: usize) {
        let mut screens = self.inner.screens.try_lock().expect("fresh, uncontended");
        if let Some(screen) = screens.get_mut(pane_id) {
            screen.restore_after_reads = reads;
        }
    }

    /// The `send_text` bytes recorded for `pane_id`, in call order -- lets a
    /// test assert exactly when (and whether) a scroller escalated.
    pub async fn sent_text_log(&self, pane_id: &str) -> Vec<String> {
        self.inner
            .sent_text
            .lock()
            .await
            .iter()
            .filter(|(p, _)| p == pane_id)
            .map(|(_, bytes)| bytes.clone())
            .collect()
    }

    /// Drive an agent's status (as a live change would).
    pub async fn set_status(&self, pane_id: &str, status: AgentStatus) -> Result<()> {
        let mut snap = self.inner.snapshot.lock().await;
        for a in &mut snap.agents {
            if a.pane_id == pane_id {
                a.status = status;
                return Ok(());
            }
        }
        Err(HerdrError::NoSuchPane(pane_id.to_string()))
    }

    fn ensure_up(&self) -> Result<()> {
        if self
            .inner
            .available
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            Ok(())
        } else {
            Err(HerdrError::Unavailable("fake herdr is down".into()))
        }
    }

    /// One `agent.start` attempt with an exact, caller-supplied `name` --
    /// no retry (`agent_start`, the trait method, owns that). Checked
    /// against the snapshot's own state, not fake-only side-state: a name
    /// collision and an unknown workspace are both read from `snap` itself,
    /// the same thing `snapshot()` returns.
    async fn agent_start_named(
        &self,
        name: &str,
        workspace_id: &str,
        cwd: Option<&str>,
        argv: &[String],
    ) -> Result<AgentStarted> {
        self.ensure_up()?;
        if argv.is_empty() {
            return Err(HerdrError::InvalidAgentArgv(
                "argv must not be empty".into(),
            ));
        }

        // herdr's agent.start does NOT resolve the workspace anchor when cwd
        // is omitted (unlike tab.create) -- it falls back to its own process
        // directory, an arbitrary folder unrelated to the workspace. Modeled
        // faithfully so a test against the fake sees the same asymmetry the
        // trait documents, not a kinder anchor-resolved path that would hide
        // the wrong-repo hazard.
        let resolved_cwd = match cwd {
            Some(c) => c.to_string(),
            None => std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "/".to_string()),
        };

        let mut snap = self.inner.snapshot.lock().await;
        // Real herdr splits into the workspace's own active tab (upstream's
        // default placement is accepted as-is) rather than creating a new
        // one -- unlike tab_create, no new Tab/PaneLayout row is needed, the
        // created pane just joins the existing tab.
        //
        // An unknown workspace is agent_placement_not_found, NOT
        // workspace_not_found: that is the exact code the live server
        // returns for a missing workspace on agent.start. tab.create keeps
        // workspace_not_found; only agent.start differs, so the fake must
        // too rather than being kinder/different from production.
        let active_tab_id = snap
            .workspaces
            .iter()
            .find(|w| w.workspace_id == workspace_id)
            .ok_or_else(|| HerdrError::Remote {
                code: "agent_placement_not_found".into(),
                message: format!("agent placement target {workspace_id} not found"),
            })?
            .active_tab_id
            .clone();

        if snap.agents.iter().any(|a| a.name == name) {
            return Err(HerdrError::AgentNameTaken {
                name: name.to_string(),
                message: format!("agent name {name} is already used"),
            });
        }

        // No active tab means genuinely nowhere to place the agent -- do
        // not invent a tab_id, that would leave a pane pointing at a tab
        // absent from snap.tabs, a shape real herdr cannot produce. This is
        // exactly what herdr itself reports as agent_placement_not_found;
        // the variant set is closed, so it rides as Remote with that code
        // rather than a new typed variant.
        let Some(tab_id) = active_tab_id else {
            return Err(HerdrError::Remote {
                code: "agent_placement_not_found".into(),
                message: format!("workspace {workspace_id} has no active tab to place an agent in"),
            });
        };

        let n = self
            .inner
            .next_created_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pane_id = format!("{workspace_id}:created-agent-pane-{n}");

        snap.agents.push(Agent {
            pane_id: pane_id.clone(),
            workspace_id: workspace_id.to_string(),
            tab_id: tab_id.clone(),
            kind: argv[0].clone(),
            name: name.to_string(),
            // Idle, not Unknown: a just-spawned agent genuinely has no work
            // in progress yet -- Unknown means "a value this app doesn't
            // recognize", which is not true here -- this is not yet a claim
            // that it finished starting.
            status: AgentStatus::Idle,
            title: String::new(),
        });
        snap.panes.push(Pane {
            pane_id: pane_id.clone(),
            workspace_id: workspace_id.to_string(),
            tab_id: tab_id.clone(),
            cwd: Some(resolved_cwd.clone()),
            foreground_cwd: Some(resolved_cwd),
        });
        drop(snap);

        // Without this, read_pane on the just-created pane returns
        // NoSuchPane -- the same seeding tab_create and FakeHerdr::new do
        // for every pane they start with.
        self.inner
            .screens
            .lock()
            .await
            .insert(pane_id.clone(), PaneScreen::new("❯ ".to_string()));

        Ok(AgentStarted {
            tab_id,
            pane_id,
            name: name.to_string(),
        })
    }
}

fn agent(pane_id: &str, kind: &str, name: &str, status: AgentStatus, title: &str) -> Agent {
    Agent {
        pane_id: pane_id.into(),
        workspace_id: pane_id.split(':').next().unwrap_or("w").into(),
        tab_id: format!("{}:t", pane_id.split(':').next().unwrap_or("w")),
        kind: kind.into(),
        name: name.into(),
        status,
        title: title.into(),
    }
}

fn pane(pane_id: &str, cwd: &str) -> Pane {
    let ws = pane_id.split(':').next().unwrap_or("w");
    Pane {
        pane_id: pane_id.into(),
        workspace_id: ws.into(),
        tab_id: format!("{ws}:t"),
        cwd: Some(cwd.into()),
        foreground_cwd: Some(cwd.into()),
    }
}

#[async_trait]
impl Herdr for FakeHerdr {
    async fn snapshot(&self) -> Result<Snapshot> {
        self.ensure_up()?;
        Ok(self.inner.snapshot.lock().await.clone())
    }

    async fn ping(&self) -> Result<ProtocolInfo> {
        self.ensure_up()?;
        Ok(ProtocolInfo {
            protocol: HERDR_PROTOCOL,
            server_version: "fake-0.7.4".into(),
        })
    }

    async fn read_pane(
        &self,
        pane_id: &str,
        source: ReadSource,
        lines: usize,
    ) -> Result<ScreenRead> {
        self.ensure_up()?;
        let mut screens = self.inner.screens.lock().await;
        match screens.get_mut(pane_id) {
            Some(screen) => {
                let text = match source {
                    ReadSource::Visible => {
                        if screen.scrolled {
                            if screen.reads_since_scroll < screen.reveal_after_reads {
                                screen.reads_since_scroll += 1;
                                screen.visible.clone() // redraw hasn't "landed" yet
                            } else {
                                screen
                                    .escape_pages
                                    .get(screen.scroll_index)
                                    .cloned()
                                    .unwrap_or_else(|| screen.visible.clone())
                            }
                        } else if screen.ever_scrolled
                            && screen.reads_since_restore < screen.restore_after_reads
                        {
                            screen.reads_since_restore += 1;
                            // still looks scrolled -- the restore hasn't "landed" yet
                            screen
                                .escape_pages
                                .get(screen.scroll_index)
                                .cloned()
                                .unwrap_or_else(|| screen.visible.clone())
                        } else {
                            screen.visible.clone()
                        }
                    }
                    ReadSource::Recent => tail_lines(&screen.recent, lines.min(1000)),
                };
                Ok(ScreenRead {
                    text,
                    revision: screen.revision,
                })
            }
            None => Err(HerdrError::NoSuchPane(pane_id.to_string())),
        }
    }

    async fn send_input(&self, pane_id: &str, text: &str, submit: bool) -> Result<()> {
        self.ensure_up()?;
        let mut screens = self.inner.screens.lock().await;
        let entry = screens
            .get_mut(pane_id)
            .ok_or_else(|| HerdrError::NoSuchPane(pane_id.to_string()))?;
        entry.visible.push_str(text);
        entry.recent.push_str(text);
        if submit {
            entry.visible.push('\n');
            entry.recent.push('\n');
        }
        entry.revision += 1; // revision bumps so a poller re-renders
        Ok(())
    }

    async fn send_keys(&self, pane_id: &str, keys: &[String]) -> Result<()> {
        self.ensure_up()?;
        let mut screens = self.inner.screens.lock().await;
        let entry = screens
            .get_mut(pane_id)
            .ok_or_else(|| HerdrError::NoSuchPane(pane_id.to_string()))?;
        // Echo keys so tests can observe them: Enter as a newline, everything
        // else as a visible <key> token.
        for k in keys {
            if k == "enter" {
                entry.visible.push('\n');
                entry.recent.push('\n');
            } else {
                let token = format!("<{k}>");
                entry.visible.push_str(&token);
                entry.recent.push_str(&token);
            }
        }
        entry.revision += 1; // revision bumps so a poller re-renders
        Ok(())
    }

    async fn send_text(&self, pane_id: &str, bytes: &str) -> Result<()> {
        self.ensure_up()?;
        // Logged regardless of whether the pane responds -- a test asserting
        // "never escalated" needs to see zero entries, and one asserting the
        // full escalate-then-restore sequence needs both bytes in order.
        self.inner
            .sent_text
            .lock()
            .await
            .push((pane_id.to_string(), bytes.to_string()));
        let mut screens = self.inner.screens.lock().await;
        let screen = screens
            .get_mut(pane_id)
            .ok_or_else(|| HerdrError::NoSuchPane(pane_id.to_string()))?;
        match bytes {
            PAGE_UP => {
                // Only a pane seeded with an escape_pages (an alt-screen
                // agent that responds to its own scroll keybinding) actually
                // changes state -- everything else generically ignores the
                // sequence (the harmless no-op case).
                if !screen.escape_pages.is_empty() {
                    if screen.scrolled {
                        // Already scrolled -- another PageUp goes one page
                        // further, clamped at the last available page (a
                        // real agent doesn't scroll past its own oldest
                        // history).
                        if screen.scroll_index + 1 < screen.escape_pages.len() {
                            screen.scroll_index += 1;
                        }
                    } else {
                        screen.scrolled = true;
                        screen.scroll_index = 0;
                        screen.ever_scrolled = true;
                    }
                    screen.reads_since_scroll = 0;
                }
            }
            RESTORE_BOTTOM => {
                screen.scrolled = false;
                screen.scroll_index = 0;
                screen.reads_since_scroll = 0;
                screen.reads_since_restore = 0;
            }
            _ => {}
        }
        Ok(())
    }

    async fn tab_create(&self, workspace_id: &str, cwd: Option<&str>) -> Result<TabCreated> {
        self.ensure_up()?;
        let n = self
            .inner
            .next_created_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tab_id = format!("{workspace_id}:created-tab-{n}");
        let pane_id = format!("{workspace_id}:created-pane-{n}");

        {
            // Actually create: append the tab, its root pane, and the
            // PaneLayout row naming it focused, all under the snapshot lock
            // (same mutate-under-lock precedent as `set_status`). `focus:
            // false` means the workspace's own `active_tab_id` is left
            // untouched -- the desktop's active tab does not move.
            let mut snap = self.inner.snapshot.lock().await;
            if !snap
                .workspaces
                .iter()
                .any(|w| w.workspace_id == workspace_id)
            {
                return Err(HerdrError::WorkspaceNotFound {
                    workspace_id: workspace_id.to_string(),
                    message: format!("no such workspace: {workspace_id}"),
                });
            }
            // With cwd omitted, herdr's tab.create resolves the workspace's
            // own anchor folder -- the safe, desktop-equivalent fallback
            // (contrast agent.start's process-dir fallback above).
            // Reproduced via the port's own anchor join so the created pane
            // lands where the real server would. A join miss degrades to
            // "/", never an empty cwd.
            let resolved_cwd = match cwd {
                Some(c) => c.to_string(),
                None => snap
                    .anchor_cwd_for_workspace(workspace_id)
                    .unwrap_or_else(|| "/".to_string()),
            };
            snap.tabs.push(Tab {
                tab_id: tab_id.clone(),
                label: "Shell".into(),
            });
            snap.panes.push(Pane {
                pane_id: pane_id.clone(),
                workspace_id: workspace_id.to_string(),
                tab_id: tab_id.clone(),
                cwd: Some(resolved_cwd.clone()),
                foreground_cwd: Some(resolved_cwd),
            });
            snap.layouts.push(PaneLayout {
                workspace_id: workspace_id.to_string(),
                tab_id: tab_id.clone(),
                focused_pane_id: Some(pane_id.clone()),
            });
        }
        // Without this, read_pane on the just-created pane returns
        // NoSuchPane -- the same seeding FakeHerdr::new does for every pane
        // it starts with.
        self.inner
            .screens
            .lock()
            .await
            .insert(pane_id.clone(), PaneScreen::new("❯ ".to_string()));

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

    #[tokio::test]
    async fn snapshot_has_all_statuses() {
        let f = FakeHerdr::new();
        let s = f.snapshot().await.unwrap();
        let st: Vec<_> = s.agents.iter().map(|a| a.status).collect();
        assert!(st.contains(&AgentStatus::Working));
        assert!(st.contains(&AgentStatus::Blocked));
        assert!(st.contains(&AgentStatus::Done));
        assert!(st.contains(&AgentStatus::Idle));
    }

    #[tokio::test]
    async fn envelope_fake_seed_joins() {
        // The seed must satisfy the same anchor shape a live snapshot does:
        // every workspace's active tab has a layout entry whose focused pane
        // really exists in panes[].
        let s = FakeHerdr::new().snapshot().await.unwrap();
        assert!(!s.workspaces.is_empty());

        for w in &s.workspaces {
            let active_tab = w
                .active_tab_id
                .as_deref()
                .unwrap_or_else(|| panic!("{} has no active_tab_id", w.workspace_id));
            assert!(
                s.tabs.iter().any(|t| t.tab_id == active_tab),
                "{active_tab} is not in tabs[]"
            );
            let layout = s
                .layouts
                .iter()
                .find(|l| l.workspace_id == w.workspace_id && l.tab_id == active_tab)
                .unwrap_or_else(|| panic!("no layout for {}/{active_tab}", w.workspace_id));
            let focused = layout.focused_pane_id.as_deref().unwrap();
            let anchor = s
                .panes
                .iter()
                .find(|p| p.pane_id == focused)
                .unwrap_or_else(|| panic!("{focused} is not in panes[]"));
            assert_eq!(anchor.workspace_id, w.workspace_id);
            assert!(anchor
                .foreground_cwd
                .as_deref()
                .or(anchor.cwd.as_deref())
                .is_some());
        }

        // At least one seeded pane is a plain shell absent from agents[] — the
        // case that makes panes[] irreplaceable by agents[].
        assert!(s
            .panes
            .iter()
            .any(|p| !s.agents.iter().any(|a| a.pane_id == p.pane_id)));
    }

    #[tokio::test]
    async fn ping_compatible() {
        assert!(FakeHerdr::new().ping().await.unwrap().is_compatible());
    }

    #[tokio::test]
    async fn read_then_reply_echoes_and_bumps_revision() {
        let f = FakeHerdr::new();
        let before = f.read_pane("w1:p1", ReadSource::Visible, 0).await.unwrap();
        f.send_input("w1:p1", "yes please", true).await.unwrap();
        let after = f.read_pane("w1:p1", ReadSource::Visible, 0).await.unwrap();
        assert!(after.text.contains("yes please"));
        assert!(after.revision > before.revision);
    }

    #[tokio::test]
    async fn send_keys_echoes_and_bumps_revision() {
        let f = FakeHerdr::new();
        let before = f.read_pane("w1:p1", ReadSource::Visible, 0).await.unwrap();
        f.send_keys("w1:p1", &["down".into(), "enter".into()])
            .await
            .unwrap();
        let after = f.read_pane("w1:p1", ReadSource::Visible, 0).await.unwrap();
        assert!(after.text.contains("<down>"));
        assert!(after.revision > before.revision);
    }

    #[tokio::test]
    async fn send_keys_unknown_pane_errors() {
        let f = FakeHerdr::new();
        assert!(matches!(
            f.send_keys("nope", &["up".into()]).await,
            Err(HerdrError::NoSuchPane(_))
        ));
    }

    #[tokio::test]
    async fn unknown_pane_errors() {
        let f = FakeHerdr::new();
        assert!(matches!(
            f.read_pane("nope", ReadSource::Visible, 0).await,
            Err(HerdrError::NoSuchPane(_))
        ));
    }

    #[tokio::test]
    async fn down_when_set() {
        let f = FakeHerdr::new();
        f.set_available(false);
        assert!(matches!(
            f.snapshot().await,
            Err(HerdrError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn tabcreate_fake_appends_tab_pane_layout_and_screen() {
        let f = FakeHerdr::new();
        let before = f.snapshot().await.unwrap();

        let created = f
            .tab_create("w1", Some("/home/dev/new-folder"))
            .await
            .unwrap();

        let after = f.snapshot().await.unwrap();
        assert_eq!(after.tabs.len(), before.tabs.len() + 1);
        assert_eq!(after.panes.len(), before.panes.len() + 1);
        assert_eq!(after.layouts.len(), before.layouts.len() + 1);

        assert!(after.tabs.iter().any(|t| t.tab_id == created.tab_id));

        let pane = after
            .panes
            .iter()
            .find(|p| p.pane_id == created.pane_id)
            .expect("created pane must be in panes[]");
        assert_eq!(pane.workspace_id, "w1");
        assert_eq!(pane.tab_id, created.tab_id);
        assert_eq!(pane.cwd.as_deref(), Some("/home/dev/new-folder"));
        assert_eq!(pane.foreground_cwd.as_deref(), Some("/home/dev/new-folder"));

        let layout = after
            .layouts
            .iter()
            .find(|l| l.workspace_id == "w1" && l.tab_id == created.tab_id)
            .expect("created tab must have a PaneLayout row");
        assert_eq!(
            layout.focused_pane_id.as_deref(),
            Some(created.pane_id.as_str())
        );

        // focus: false -- the workspace's own active tab does not move.
        let ws = after
            .workspaces
            .iter()
            .find(|w| w.workspace_id == "w1")
            .unwrap();
        assert_eq!(ws.active_tab_id.as_deref(), Some("w1:t"));

        // The screens entry is what makes the created pane readable at all.
        assert!(f
            .read_pane(&created.pane_id, ReadSource::Visible, 0)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn tabcreate_fake_unknown_workspace_errors() {
        let f = FakeHerdr::new();
        match f.tab_create("no-such-workspace", Some("/tmp")).await {
            Err(HerdrError::WorkspaceNotFound { workspace_id, .. }) => {
                assert_eq!(workspace_id, "no-such-workspace");
            }
            other => panic!("expected WorkspaceNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tabcreate_created_pane_is_readable() {
        let f = FakeHerdr::new();
        let created = f
            .tab_create("w2", Some("/home/dev/new-shell"))
            .await
            .unwrap();

        let screen = f
            .read_pane(&created.pane_id, ReadSource::Visible, 0)
            .await
            .expect("newly created pane must be readable, not NoSuchPane");
        assert_eq!(screen.text, "❯ ");
    }

    #[tokio::test]
    async fn agentstart_fake_appends_named_agent_and_readable_pane() {
        let f = FakeHerdr::new();
        let before = f.snapshot().await.unwrap();

        let started = f
            .agent_start_named(
                "mobile-agent-1",
                "w1",
                Some("/home/dev/new-agent"),
                &["claude".to_string()],
            )
            .await
            .unwrap();

        let after = f.snapshot().await.unwrap();
        assert_eq!(after.agents.len(), before.agents.len() + 1);
        assert_eq!(after.panes.len(), before.panes.len() + 1);

        let agent = after
            .agents
            .iter()
            .find(|a| a.pane_id == started.pane_id)
            .expect("started agent must be in agents[]");
        assert_eq!(agent.name, "mobile-agent-1");
        assert_eq!(agent.workspace_id, "w1");

        let pane = after
            .panes
            .iter()
            .find(|p| p.pane_id == started.pane_id)
            .expect("started agent's pane must be in panes[]");
        assert_eq!(pane.cwd.as_deref(), Some("/home/dev/new-agent"));
        assert_eq!(pane.foreground_cwd.as_deref(), Some("/home/dev/new-agent"));

        // The screens entry is what makes the created pane readable at all.
        assert!(f
            .read_pane(&started.pane_id, ReadSource::Visible, 0)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn agentstart_duplicate_name_errors() {
        let f = FakeHerdr::new();
        f.agent_start_named("dup-name", "w1", Some("/home/dev"), &["claude".to_string()])
            .await
            .unwrap();

        match f
            .agent_start_named("dup-name", "w2", Some("/home/dev"), &["codex".to_string()])
            .await
        {
            Err(HerdrError::AgentNameTaken { name, .. }) => assert_eq!(name, "dup-name"),
            other => panic!("expected AgentNameTaken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn agentstart_empty_argv_errors() {
        let f = FakeHerdr::new();
        let before = f.snapshot().await.unwrap();

        let err = f
            .agent_start_named("mobile-agent-1", "w1", Some("/home/dev"), &[])
            .await
            .unwrap_err();
        assert!(matches!(err, HerdrError::InvalidAgentArgv(_)));

        // Nothing was mutated -- an invalid request creates nothing.
        let after = f.snapshot().await.unwrap();
        assert_eq!(after.agents.len(), before.agents.len());
    }

    #[tokio::test]
    async fn createcwd_agentstart_unknown_workspace_is_placement_not_found() {
        // agent.start on an unknown workspace returns the SAME code the live
        // server does -- agent_placement_not_found (Remote), NOT
        // WorkspaceNotFound. The old fake returned WorkspaceNotFound, a
        // variant kinder and different from production. tab.create keeps
        // WorkspaceNotFound; only agent.start differs.
        let f = FakeHerdr::new();
        match f
            .agent_start_named(
                "mobile-agent-1",
                "no-such-workspace",
                Some("/home/dev"),
                &["claude".to_string()],
            )
            .await
        {
            Err(HerdrError::Remote { code, .. }) => {
                assert_eq!(code, "agent_placement_not_found");
            }
            other => panic!("expected Remote(agent_placement_not_found), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn agentstart_no_active_tab_errors_without_inventing_one() {
        // A workspace with no active tab has genuinely nowhere to place the
        // agent -- must not invent a tab_id (that would leave a pane
        // pointing at a tab absent from tabs[], a shape real herdr cannot
        // produce). herdr itself reports this as agent_placement_not_found.
        let f = FakeHerdr::new();
        {
            let mut snap = f.inner.snapshot.lock().await;
            snap.workspaces.push(Workspace {
                workspace_id: "w-no-tab".into(),
                label: "no-active-tab".into(),
                agent_status: AgentStatus::Unknown,
                active_tab_id: None,
            });
        }

        let before = f.snapshot().await.unwrap();
        let screens_before = f.inner.screens.lock().await.len();
        match f
            .agent_start_named(
                "mobile-agent-1",
                "w-no-tab",
                Some("/home/dev"),
                &["claude".to_string()],
            )
            .await
        {
            Err(HerdrError::Remote { code, .. }) => {
                assert_eq!(code, "agent_placement_not_found");
            }
            other => panic!("expected Remote(agent_placement_not_found), got {other:?}"),
        }

        // Nothing was mutated -- a placement failure creates nothing: no
        // agent, no pane, no screens entry.
        let after = f.snapshot().await.unwrap();
        assert_eq!(after.agents.len(), before.agents.len());
        assert_eq!(after.panes.len(), before.panes.len());
        assert_eq!(f.inner.screens.lock().await.len(), screens_before);
    }

    #[tokio::test]
    async fn agentstart_port_retries_transparently_on_collision() {
        // The public trait method must never surface AgentNameTaken to its
        // caller for an ordinary collision -- it retries with a new
        // auto-generated name and succeeds.
        let f = FakeHerdr::new();
        // Every seeded demo agent name is distinct from whatever
        // generate_agent_name() produces, so this call should simply
        // succeed on the first attempt -- proving the public entry point
        // works end to end, not just the exact-name helper.
        let started = f
            .agent_start("w1", Some("/home/dev/new-agent"), &["claude".to_string()])
            .await
            .unwrap();
        assert!(!started.name.is_empty());

        let snap = f.snapshot().await.unwrap();
        assert!(snap.agents.iter().any(|a| a.pane_id == started.pane_id));
    }

    #[tokio::test]
    async fn createcwd_fake_seed_has_shell_only_workspace_with_cwd_only_anchor() {
        // The live client produces two shapes the old seed could not: a
        // workspace with NO agents, and an anchor pane whose foreground_cwd is
        // absent (cwd-only). The seed must carry both so later cells can
        // exercise a shell-only destination and the cwd-fallback path.
        let s = FakeHerdr::new().snapshot().await.unwrap();

        let shell_only = s
            .workspaces
            .iter()
            .find(|w| !s.agents.iter().any(|a| a.workspace_id == w.workspace_id))
            .expect("seed must contain a workspace with no agents");

        // Its anchor resolves from cwd, not foreground_cwd -- the cwd-only
        // shape (foreground_cwd absent).
        let anchor = s
            .anchor_for_workspace(&shell_only.workspace_id)
            .expect("shell-only workspace still resolves its anchor");
        assert!(
            !anchor.live,
            "anchor must come from cwd (foreground_cwd absent), not the live dir"
        );

        let active_tab = shell_only.active_tab_id.as_deref().unwrap();
        let focused = s
            .layouts
            .iter()
            .find(|l| l.workspace_id == shell_only.workspace_id && l.tab_id == active_tab)
            .unwrap()
            .focused_pane_id
            .as_deref()
            .unwrap();
        let pane = s.panes.iter().find(|p| p.pane_id == focused).unwrap();
        assert!(pane.cwd.is_some(), "anchor pane has cwd set");
        assert!(
            pane.foreground_cwd.is_none(),
            "anchor pane's foreground_cwd is absent -- the cwd-only shape"
        );
    }

    #[tokio::test]
    async fn createcwd_fake_tab_create_omitted_cwd_resolves_workspace_anchor() {
        // tab.create with cwd omitted resolves the workspace's OWN anchor
        // folder -- the safe, desktop-equivalent fallback -- so the created
        // pane lands in the workspace's directory, never an empty cwd.
        let f = FakeHerdr::new();
        let anchor = f
            .snapshot()
            .await
            .unwrap()
            .anchor_cwd_for_workspace("w3")
            .unwrap();

        let created = f.tab_create("w3", None).await.unwrap();

        let after = f.snapshot().await.unwrap();
        let pane = after
            .panes
            .iter()
            .find(|p| p.pane_id == created.pane_id)
            .unwrap();
        assert_eq!(
            pane.cwd.as_deref(),
            Some(anchor.as_str()),
            "omitted cwd must resolve the workspace anchor, not an empty/process dir"
        );
    }

    #[tokio::test]
    async fn createcwd_fake_agent_start_omitted_cwd_uses_process_dir_not_anchor() {
        // The asymmetry the trait documents: agent.start does NOT resolve the
        // workspace anchor when cwd is omitted -- it falls back to the process
        // directory, an arbitrary folder that is NOT the workspace's anchor.
        // This is exactly why a caller that cannot resolve a real path must
        // refuse rather than omit cwd here.
        let f = FakeHerdr::new();
        let anchor = f
            .snapshot()
            .await
            .unwrap()
            .anchor_cwd_for_workspace("w3")
            .unwrap();

        let started = f
            .agent_start_named("mobile-agent-omit", "w3", None, &["claude".to_string()])
            .await
            .unwrap();

        let after = f.snapshot().await.unwrap();
        let pane = after
            .panes
            .iter()
            .find(|p| p.pane_id == started.pane_id)
            .unwrap();
        assert!(
            pane.cwd.is_some(),
            "still lands in some real dir (process cwd)"
        );
        assert_ne!(
            pane.cwd.as_deref(),
            Some(anchor.as_str()),
            "agent.start must NOT resolve the workspace anchor -- unlike tab.create"
        );
    }
}
