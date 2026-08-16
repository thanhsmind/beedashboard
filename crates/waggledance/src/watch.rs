//! Filesystem watcher: notify-debouncer-full (200ms) → incremental reindex →
//! broadcast a reload-signal. Watches each project known at daemon start
//! (PRD FR-08/FR-09/FR-09b).

use anyhow::Result;
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use waggledance_core::Engine;

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
///
/// A path is only included when it actually changed (D2, `backlog-groom-1`):
/// a touch / byte-identical rewrite reindexes but reports no change, so it
/// broadcasts no reload. Deletions and brand-new paths always count as
/// changed, unchanged from before.
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
        let content_changed = if path.exists() {
            // Reindex the file and refresh its outgoing links (keeps backlinks
            // live). Only a genuine content change reports true.
            engine
                .index_file_incremental(project, path)
                .unwrap_or(false)
        } else {
            // Removed/renamed away — drop from index (survives atomic-save because
            // the debounced batch also carries the recreated path).
            let _ = engine.remove_file(project, path);
            true
        };
        if content_changed {
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

#[cfg(test)]
mod tests {
    use super::*;
    use waggledance_core::{Config, Engine, SqliteStore};

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    /// D2 (`backlog-groom-1`): a byte-identical reindex reports no change, a
    /// real content edit and a brand-new path each do.
    #[test]
    fn reindex_paths_reports_change_only_when_content_actually_changed() {
        let dir = std::env::temp_dir().join(format!("waggledance-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\nfirst");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        let a = dir.join("docs/a.md");

        // The initial register() already indexed docs/a.md via its full scan
        // (same content on disk), so reindexing it again unchanged emits nothing.
        let changed = reindex_paths(&engine, std::slice::from_ref(&a));
        assert!(
            changed.is_empty(),
            "byte-identical reindex must emit no reload, got {changed:?}"
        );

        // A real content edit reports a reload for that path.
        std::fs::write(&a, "# A\nsecond").unwrap();
        let changed = reindex_paths(&engine, std::slice::from_ref(&a));
        assert_eq!(changed, vec![format!("{}/docs/a.md", project.id)]);

        // A brand-new path (no prior stored content) reports a reload too.
        write(&dir, "docs/b.md", "# B\nbrand new");
        let b = dir.join("docs/b.md");
        let changed = reindex_paths(&engine, &[b]);
        assert_eq!(changed, vec![format!("{}/docs/b.md", project.id)]);

        // Re-reindexing the same new file unchanged now emits nothing.
        let b = dir.join("docs/b.md");
        let changed = reindex_paths(&engine, &[b]);
        assert!(
            changed.is_empty(),
            "reindexing the now-stored new file unchanged must emit no reload, got {changed:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Deletions keep reporting as changed (removal always drives a reload).
    #[test]
    fn reindex_paths_still_reports_a_deleted_path_as_changed() {
        let dir =
            std::env::temp_dir().join(format!("waggledance-watch-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\ncontent");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        let a = dir.join("docs/a.md");
        std::fs::remove_file(&a).unwrap();

        let changed = reindex_paths(&engine, &[a]);
        assert_eq!(changed, vec![format!("{}/docs/a.md", project.id)]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
