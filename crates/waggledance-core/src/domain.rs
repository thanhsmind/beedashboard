//! Domain types. Pure data — no dependency on Axum/Tauri/SQLite.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A registered project: a root directory whose markdown tree is indexed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: PathBuf,
    /// RFC3339 timestamps.
    pub created_at: String,
    pub last_seen_at: String,
    /// D6 opt-in: true only when this project has been switched into
    /// orchestrator-dispatch mode. Effective only when `terminal.enabled` is
    /// also on (the caller combines the two — see `Engine::orchestration_allowed`).
    pub orchestration_enabled: bool,
}

/// One indexed markdown file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedFile {
    pub project_id: String,
    /// Absolute path on disk.
    pub abs_path: PathBuf,
    /// Path relative to project root — used as the URL segment.
    pub rel_path: String,
    /// First H1, or filename if none.
    pub title: String,
    pub size_bytes: u64,
    /// RFC3339 modified timestamp.
    pub modified_at: String,
}

/// A heading extracted from a file (for TOC / anchor navigation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub slug: String,
}

/// A resolved internal link, ready to become an `<a href>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLink {
    /// The rewritten in-app URL, or None if the link is broken/unresolvable.
    pub url: Option<String>,
    /// True when the target could not be resolved within the project.
    pub broken: bool,
}

/// Result of a search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub project_id: String,
    pub rel_path: String,
    pub title: String,
    pub excerpt: String,
    pub url: String,
    pub score: f64,
}

/// One orchestrator dispatch: a task sent to one pane, from baseline capture
/// to a terminal status. Durable per D7 so a restarted orchestrator recovers
/// the fleet by reading run state instead of carrying a prompt-side roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub project_id: String,
    pub pane_id: String,
    /// The preset used to spawn the pane, or `None` when `dispatch` targeted
    /// an already-running pane instead of spawning one (D3).
    pub preset_label: Option<String>,
    pub task: String,
    /// Pre-send transcript capture the run's delta and marker-freshness are
    /// measured against (D5).
    pub baseline: String,
    /// The split completion token minted for this run (D5).
    pub marker: String,
    /// Protocol status enum string: pending/working/done/blocked/failed/timeout.
    pub status: String,
    /// RFC3339 timestamps.
    pub created_at: String,
    pub updated_at: String,
}

/// Rendered markdown page plus metadata for the viewer.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub html: String,
    pub title: String,
    pub headings: Vec<Heading>,
    /// True if the page contains mermaid blocks (client must load mermaid.js).
    pub has_mermaid: bool,
    /// The raw markdown source, carried so the viewer can map a DOM selection
    /// back to source lines (copy-as-markdown) via the `data-sourcepos` attrs.
    pub source: String,
}
