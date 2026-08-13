//! Filesystem watcher: notify-debouncer-full (200ms) → incremental reindex →
//! broadcast a reload-signal. Watches each project known at daemon start
//! (PRD FR-08/FR-09/FR-09b).

use anyhow::Result;
use waggledance_core::Engine;
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub type WatchHandle = Debouncer<notify::RecommendedWatcher, FileIdMap>;

/// Build a debouncer watching every registered project. The returned handle
/// must be kept alive for the daemon's lifetime.
pub fn spawn_watchers(
    engine: Arc<Engine>,
    reload_tx: broadcast::Sender<String>,
) -> Result<WatchHandle> {
    let debounce = Duration::from_millis(engine.config.indexing.debounce_ms.max(50));
    let cb_engine = engine.clone();

    let mut debouncer = new_debouncer(debounce, None, move |res: DebounceEventResult| {
        if let Ok(events) = res {
            let paths: Vec<_> = events.into_iter().flat_map(|e| e.paths.clone()).collect();
            let changed = reindex_paths(&cb_engine, &paths);
            if !changed.is_empty() {
                let payload = serde_json::json!({ "changed": changed }).to_string();
                let _ = reload_tx.send(payload);
            }
        }
    })?;

    for project in engine.list_projects().unwrap_or_default() {
        let root = project.root_path.clone();
        if root.exists() {
            debouncer
                .watcher()
                .watch(&root, RecursiveMode::Recursive)
                .ok();
            debouncer.cache().add_root(&root, RecursiveMode::Recursive);
        }
    }
    Ok(debouncer)
}

/// Reindex the given paths incrementally. Returns the changed documents as
/// `<project_id>/<repo-relative-path>` entries (slash-separated on every
/// platform) — the reload payload clients match their own URL against.
fn reindex_paths(engine: &Engine, paths: &[std::path::PathBuf]) -> Vec<String> {
    let projects = engine.list_projects().unwrap_or_default();
    let mut changed = Vec::new();

    for path in paths {
        if !is_markdown(path) {
            continue;
        }
        let Some(project) = projects.iter().find(|p| path.starts_with(&p.root_path)) else {
            continue;
        };
        let indexed = if path.exists() {
            // Reindex the file and refresh its outgoing links (keeps backlinks live).
            engine.index_file_incremental(project, path).is_ok()
        } else {
            // Removed/renamed away — drop from index (survives atomic-save because
            // the debounced batch also carries the recreated path).
            let _ = engine.remove_file(project, path);
            true
        };
        if indexed {
            if let Ok(rel) = path.strip_prefix(&project.root_path) {
                let rel = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                changed.push(format!("{}/{}", project.id, rel));
            }
        }
    }
    changed
}

fn is_markdown(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("md") | Some("markdown")
    )
}
