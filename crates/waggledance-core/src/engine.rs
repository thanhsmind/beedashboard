//! Application core: the facade the HTTP/MCP/CLI adapters call. Owns the store,
//! config, and renderer, and implements the high-level use cases (view_file,
//! render, search, registry) — including implicit project auto-create (FR-04).

use crate::code_source::{self, DirListing, SourceContent};
use crate::config::Config;
use crate::domain::{IndexedFile, Project, RenderedPage, SearchResult};
use crate::error::{Error, Result};
use crate::fuzzy::{self, FuzzyHit};
use crate::indexer::{self, IndexService};
use crate::render::{self, HighlightedSource, RenderService};
use crate::repository::SqliteStore;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub struct Engine {
    pub store: SqliteStore,
    pub config: Config,
    render: RenderService,
}

#[derive(Debug, Clone)]
pub struct ViewFile {
    pub url: String,
    pub project_id: String,
    pub rel_path: String,
    /// Short code for this file — the `<code>` in `/s/<code>`.
    pub code: String,
}

impl Engine {
    pub fn new(store: SqliteStore, config: Config) -> Self {
        Self {
            store,
            config,
            render: RenderService::new(),
        }
    }

    fn max_bytes(&self) -> u64 {
        self.config
            .indexing
            .max_file_size_mb
            .saturating_mul(1024 * 1024)
    }

    /// Canonicalize when possible; otherwise fall back to the given path.
    fn canonical(root: &Path) -> PathBuf {
        std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
    }

    /// Find the project owning `root`, or create + index it (implicit registration).
    pub fn ensure_project(&self, root: &Path, name: Option<&str>) -> Result<Project> {
        let root = Self::canonical(root);
        if let Some(mut p) = self.store.find_project_by_root(&root)? {
            p.last_seen_at = indexer::now_rfc3339();
            self.store.upsert_project(&p)?;
            return Ok(p);
        }
        let id = self.unique_id(&indexer::slug_from_root(&root))?;
        let name = name.map(|s| s.to_string()).unwrap_or_else(|| {
            root.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&id)
                .to_string()
        });
        let now = indexer::now_rfc3339();
        let project = Project {
            id,
            name,
            root_path: root,
            created_at: now.clone(),
            last_seen_at: now,
        };
        self.store.upsert_project(&project)?;
        IndexService::index_project(
            &self.store,
            &project,
            &self.config.indexing.exclude_patterns,
            self.max_bytes(),
        )?;
        self.reindex_links(&project)?;
        Ok(project)
    }

    fn unique_id(&self, base: &str) -> Result<String> {
        if self.store.get_project(base)?.is_none() {
            return Ok(base.to_string());
        }
        for n in 2..10_000 {
            let cand = format!("{base}-{n}");
            if self.store.get_project(&cand)?.is_none() {
                return Ok(cand);
            }
        }
        Err(Error::Other("could not allocate project id".into()))
    }

    /// The core `waggledance_view_file` use case: ensure project, index the file now,
    /// return its app URL.
    pub fn view_file(&self, project_root: &Path, rel_path: &str) -> Result<ViewFile> {
        let project = self.ensure_project(project_root, None)?;
        let abs = project.root_path.join(rel_path);
        let abs = crate::link_resolver::normalize(&abs);
        self.index_file_incremental(&project, &abs)?;
        let rel = indexer::rel_path_str(&project.root_path, &abs);
        if rel.is_empty() {
            return Err(Error::PathOutsideProject(abs));
        }
        let code = crate::short_link::short_code(&crate::short_link::path_hash(&project.id, &rel));
        Ok(ViewFile {
            url: format!("/p/{}/{}", project.id, rel),
            project_id: project.id,
            rel_path: rel,
            code,
        })
    }

    /// Register a project explicitly (CLI). Same as ensure_project + optional name.
    pub fn register(&self, root: &Path, name: Option<&str>) -> Result<Project> {
        self.ensure_project(root, name)
    }

    pub fn unregister(&self, project_id: &str) -> Result<()> {
        self.store.delete_project(project_id)
    }

    /// Full re-scan of a project to reconcile drift (FR-09b).
    pub fn refresh(&self, project_id: &str) -> Result<usize> {
        let project = self
            .store
            .get_project(project_id)?
            .ok_or_else(|| Error::ProjectNotFound(project_id.to_string()))?;
        let n = IndexService::index_project(
            &self.store,
            &project,
            &self.config.indexing.exclude_patterns,
            self.max_bytes(),
        )?;
        self.reindex_links(&project)?;
        Ok(n)
    }

    /// Selective re-index of files whose fs mtime or size differs from the
    /// stored `files` row (D4): re-reads/upserts only what changed, indexes
    /// new files, and removes files that vanished from disk. Reuses the same
    /// walk (`scan_markdown_files`) and per-file indexing
    /// (`IndexService::index_file`/`remove_file`) `index_project` uses, so
    /// there is exactly one walk/index code path.
    ///
    /// Returns the number of files actually re-read (excludes untouched
    /// files, which are compared by metadata only — never content-read).
    ///
    /// Two guards keep a bad or empty scan from being read as "everything
    /// was deleted" (plan.md Approach 1b, review finding 8): a project whose
    /// `root_path` no longer exists on disk short-circuits before any delete
    /// happens, and a walk that finds zero files against a non-empty index
    /// skips the delete pass entirely (a permissions glitch or unmounted
    /// root must never empty the index).
    ///
    /// Delete criterion (review finding P1-1): a row not seen by the walk is
    /// deleted only when `fs::metadata` on its `abs_path` fails with
    /// `NotFound`, never merely because the walk didn't see it. The walk
    /// respects `.gitignore` and `exclude_patterns` while `view_file`
    /// indexes a single file without that filter, so a gitignored/excluded
    /// file that got indexed via `view_file` must survive this pass; and the
    /// walk silently drops per-entry errors, so an unreadable subtree must
    /// not be mistaken for a deleted one. Any stat outcome other than
    /// `NotFound` (the file exists, or the stat itself errored for another
    /// reason such as permission-denied) keeps the row.
    ///
    /// Skip rule for files `index_file` would decline to store (finding 9):
    /// an oversize file is skipped by the same `size > max_bytes` metadata
    /// check `index_file` itself gates on, so its content is never read here
    /// either; an unreadable path fails at the metadata stat, before any
    /// read is attempted. A file whose content turns out to be invalid UTF-8
    /// still costs one read attempt per stale-check while it lacks a stored
    /// row — remembering that failure across calls would need a persisted
    /// marker (a schema change out of scope here), so this is the cheapest
    /// correct rule without one.
    pub fn refresh_stale(&self, project_id: &str) -> Result<usize> {
        let project = self
            .store
            .get_project(project_id)?
            .ok_or_else(|| Error::ProjectNotFound(project_id.to_string()))?;

        if !project.root_path.exists() {
            return Ok(0);
        }

        let max_bytes = self.max_bytes();
        let found = indexer::scan_markdown_files(
            &project.root_path,
            &self.config.indexing.exclude_patterns,
        );
        let existing = self.store.list_files(&project.id)?;

        let mut seen: HashSet<String> = HashSet::new();
        let mut n = 0usize;
        for abs in &found {
            let rel = indexer::rel_path_str(&project.root_path, abs);
            if rel.is_empty() {
                continue;
            }
            seen.insert(rel.clone());

            // Metadata-only stat: same cost class as the mtime/size we
            // already store, no content read.
            let meta = match std::fs::metadata(abs) {
                Ok(m) => m,
                Err(_) => continue, // unreadable — no read attempted
            };
            if meta.len() > max_bytes {
                continue; // oversize — mirrors index_file's own gate, no read
            }
            // MUST format through the identical path index_file uses
            // (indexer.rs), or every file compares as changed (D4 review
            // finding 10).
            let modified_at = meta
                .modified()
                .ok()
                .and_then(|t| OffsetDateTime::from(t).format(&Rfc3339).ok())
                .unwrap_or_default();

            let stale = match existing.iter().find(|f| f.rel_path == rel) {
                None => true,
                Some(f) => f.size_bytes != meta.len() || f.modified_at != modified_at,
            };
            if !stale {
                continue;
            }
            if IndexService::index_file(&self.store, &project, abs, max_bytes)?.is_some() {
                self.compute_file_links(&project, abs)?;
                n += 1;
            }
        }

        // Delete criterion: stat-confirmed missing, never walk-absence. The
        // walk (`scan_markdown_files`) honors .gitignore and
        // `exclude_patterns`, but `view_file` indexes a single file WITHOUT
        // that filter (`index_file_incremental`) — so a gitignored or
        // excluded file can be indexed and then show up here as merely
        // "not walked", not "gone". And `walker.flatten()` in the scan
        // silently drops per-entry walk errors, so an unreadable subtree
        // would otherwise look identical to a deleted one. A row is removed
        // only when `fs::symlink_metadata` on its `abs_path` fails with
        // `NotFound`; any other outcome — the file exists (gitignored /
        // excluded / just outside this walk) or the stat itself failed for
        // some other reason (permission denied, transient IO) — keeps the
        // row, erring toward "still there" rather than un-publishing it.
        let vanished_root_scan = found.is_empty() && !existing.is_empty();
        if !vanished_root_scan {
            for f in &existing {
                if seen.contains(&f.rel_path) {
                    continue;
                }
                let truly_missing = matches!(
                    std::fs::metadata(&f.abs_path),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound
                );
                if truly_missing {
                    self.remove_file(&project, &f.abs_path)?;
                }
            }
        }

        Ok(n)
    }

    /// Index a single file and (re)compute its outgoing links. Used by view_file
    /// and the filesystem watcher.
    ///
    /// Returns whether the file's content actually changed relative to what was
    /// previously stored (D2, `backlog-groom-1`): the live-reload watcher uses
    /// this to skip broadcasting a reload for a touch / byte-identical rewrite.
    /// A path with no prior stored content (brand-new, or unreadable before)
    /// always counts as changed. The compare happens against the content read
    /// just before this call overwrites it — no extra hashing pass, a plain
    /// blob compare against the `files_fts.content` column the indexer already
    /// writes.
    pub fn index_file_incremental(&self, project: &Project, abs: &Path) -> Result<bool> {
        let rel = indexer::rel_path_str(&project.root_path, abs);
        let previous = if rel.is_empty() {
            None
        } else {
            self.store.file_content(&project.id, &rel)?
        };
        let new_content = std::fs::read_to_string(abs).ok();
        let indexed = IndexService::index_file(&self.store, project, abs, self.max_bytes())?;
        self.compute_file_links(project, abs)?;
        let changed = match (&indexed, &new_content) {
            (Some(_), Some(c)) => previous.as_deref() != Some(c.as_str()),
            // Indexed despite our own read failing (a race with the writer) —
            // treat conservatively as changed so a real edit is never dropped.
            (Some(_), None) => true,
            // Skipped by IndexService (too big / unreadable) — nothing in the
            // index moved, so there is nothing to reload for.
            (None, _) => false,
        };
        Ok(changed)
    }

    /// Drop a file from the index (and its outgoing links).
    pub fn remove_file(&self, project: &Project, abs: &Path) -> Result<()> {
        IndexService::remove_file(&self.store, project, abs)
    }

    /// Resolve and store the internal links a single file points to.
    fn compute_file_links(&self, project: &Project, abs: &Path) -> Result<()> {
        let rel = indexer::rel_path_str(&project.root_path, abs);
        if rel.is_empty() {
            return Ok(());
        }
        let content = std::fs::read_to_string(abs).unwrap_or_default();
        let index = self.store.file_abs_paths(&project.id)?;
        let targets = render::extract_internal_links(&content, abs, &project.root_path, &index);
        self.store.set_file_links(&project.id, &rel, &targets)
    }

    /// Recompute links for every file in a project (after a full scan).
    fn reindex_links(&self, project: &Project) -> Result<()> {
        let files = self.store.list_files(&project.id)?;
        let index = self.store.file_abs_paths(&project.id)?;
        for f in files {
            let content = std::fs::read_to_string(&f.abs_path).unwrap_or_default();
            let targets =
                render::extract_internal_links(&content, &f.abs_path, &project.root_path, &index);
            self.store
                .set_file_links(&project.id, &f.rel_path, &targets)?;
        }
        Ok(())
    }

    /// Files that link to `rel_path` → (source_rel, title). FR-18 backlinks.
    pub fn backlinks(&self, project_id: &str, rel_path: &str) -> Result<Vec<(String, String)>> {
        self.store.backlinks(project_id, rel_path)
    }

    /// Render a file for the viewer, rewriting internal links against the index.
    pub fn render_file(&self, project_id: &str, rel_path: &str) -> Result<RenderedPage> {
        let project = self
            .store
            .get_project(project_id)?
            .ok_or_else(|| Error::ProjectNotFound(project_id.to_string()))?;
        let file = self
            .store
            .get_file(project_id, rel_path)?
            .ok_or_else(|| Error::FileNotFound(rel_path.to_string()))?;
        let content = std::fs::read_to_string(&file.abs_path)?;
        let index = self.store.file_abs_paths(project_id)?;
        Ok(self.render.render(
            &content,
            &file.abs_path,
            project_id,
            &project.root_path,
            &index,
        ))
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.store.list_projects()
    }

    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        self.store.get_project(id)
    }

    pub fn list_files(&self, project_id: &str) -> Result<Vec<IndexedFile>> {
        self.store.list_files(project_id)
    }

    pub fn file_count(&self, project_id: &str) -> Result<usize> {
        self.store.file_count(project_id)
    }

    pub fn search(
        &self,
        query: &str,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        self.store.search(query, project_id, limit)
    }

    /// Fuzzy file-jump: rank a project's files by a fuzzy match of `query`
    /// against their relative paths (name/path jump, complementing the
    /// content-based `search`). Ordered by descending match score.
    pub fn fuzzy_files(
        &self,
        project_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FuzzyHit>> {
        let files = self.store.list_files(project_id)?;
        Ok(fuzzy::rank_files(&files, project_id, query, limit))
    }

    /// Resolve an on-disk absolute path for an asset/image request, guarding
    /// against path traversal (must stay within the project root), a
    /// safe-extension allowlist, and configured exclude patterns.
    pub fn asset_path(&self, project_id: &str, rel_path: &str) -> Result<PathBuf> {
        let project = self
            .store
            .get_project(project_id)?
            .ok_or_else(|| Error::ProjectNotFound(project_id.to_string()))?;
        let joined = crate::link_resolver::normalize(&project.root_path.join(rel_path));
        let canonical = std::fs::canonicalize(&joined).unwrap_or(joined);
        if !canonical.starts_with(&project.root_path) {
            return Err(Error::PathOutsideProject(canonical));
        }
        // Extension check runs on `canonical` (post symlink-resolution), never
        // on `rel_path`/the URL segment: a symlink named e.g. pretty.png can
        // point at an arbitrary file, and only the resolved target's real
        // extension is trustworthy.
        if !has_allowed_asset_extension(&canonical) {
            return Err(Error::PathOutsideProject(canonical));
        }
        // Exclude-pattern check mirrors scan_markdown_files's semantics
        // (indexer.rs): exact component-name equality, not glob/substring.
        // Matched against canonical-stripped-of-root components (same
        // post-resolution path already used above) rather than the raw
        // rel_path, and never against the full absolute canonical path
        // (which would false-positive-exclude a project root that happens to
        // sit under a directory literally named one of the patterns).
        let rel = indexer::rel_path_str(&project.root_path, &canonical);
        if is_excluded_path(&rel, &self.config.indexing.exclude_patterns) {
            return Err(Error::PathOutsideProject(canonical));
        }
        Ok(canonical)
    }

    /// Resolve a Code-section request: a directory listing, a highlighted
    /// text file, or a binary notice. Every filesystem access goes through
    /// `code_source` (never `asset_path`'s extension allowlist — the Code
    /// section serves arbitrary text, so identity of the file is what's
    /// gated, not its extension). The caller (HTTP layer) never touches
    /// `code_source` or the renderer directly; both are private to `Engine`.
    pub fn code_path(&self, project_id: &str, rel_path: &str) -> Result<CodeView> {
        let project = self
            .store
            .get_project(project_id)?
            .ok_or_else(|| Error::ProjectNotFound(project_id.to_string()))?;
        let exclude = &self.config.indexing.exclude_patterns;
        let abs = code_source::resolve_source_path(&project.root_path, rel_path, exclude)?;
        if abs.is_dir() {
            let listing = code_source::list_dir(&project.root_path, rel_path, exclude)?;
            return Ok(CodeView::Dir(listing));
        }
        match code_source::read_source(&abs)? {
            SourceContent::Binary { size } => Ok(CodeView::Binary { size }),
            SourceContent::Text { text, truncated } => {
                let size = text.len() as u64;
                let highlighted = self.render.highlight_source(&abs, &text);
                Ok(CodeView::File {
                    highlighted,
                    truncated,
                    size,
                })
            }
        }
    }
}

/// Result of resolving a Code-section path — see `Engine::code_path`.
pub enum CodeView {
    Dir(DirListing),
    File {
        highlighted: HighlightedSource,
        truncated: bool,
        size: u64,
    },
    Binary {
        size: u64,
    },
}

/// Extensions asset_path serves. Mirrors the 9 tokens
/// `crates/waggledance/src/server.rs::content_type()` already recognizes;
/// waggledance-core cannot import across the crate boundary, so keep this list in
/// sync if content_type() ever changes.
const ALLOWED_ASSET_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp", "pdf",
];

fn has_allowed_asset_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .map(|e| ALLOWED_ASSET_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or(false)
}

/// True if any path component (by exact name equality) matches an exclude
/// pattern, mirroring `indexer::scan_markdown_files`'s filter semantics.
fn is_excluded_path(rel: &str, exclude_patterns: &[String]) -> bool {
    Path::new(rel)
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .any(|name| exclude_patterns.iter().any(|ex| ex == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn view_file_auto_creates_project_and_returns_url() {
        let dir = std::env::temp_dir().join(format!("waggledance-eng-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(
            &dir,
            "docs/architecture.md",
            "# Arch\nsee [api](../src/api/README.md)",
        );
        write(&dir, "src/api/README.md", "# API");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let vf = engine.view_file(&dir, "docs/architecture.md").unwrap();
        assert!(vf.url.starts_with("/p/"));
        assert!(vf.url.ends_with("/docs/architecture.md"));

        // project auto-created + fully scanned (both files indexed)
        assert_eq!(engine.file_count(&vf.project_id).unwrap(), 2);

        // rendering rewrites the cross-folder link
        let page = engine
            .render_file(&vf.project_id, "docs/architecture.md")
            .unwrap();
        assert!(page
            .html
            .contains(&format!("/p/{}/src/api/README.md", vf.project_id)));

        // second call reuses the same project id
        let vf2 = engine.view_file(&dir, "src/api/README.md").unwrap();
        assert_eq!(vf.project_id, vf2.project_id);

        // backlinks: architecture.md links to the API readme (FR-18)
        let back = engine
            .backlinks(&vf.project_id, "src/api/README.md")
            .unwrap();
        assert!(
            back.iter().any(|(rel, _)| rel == "docs/architecture.md"),
            "backlinks: {back:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn index_file_incremental_reports_changed_only_when_content_differs() {
        let dir = std::env::temp_dir().join(format!("waggledance-incr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Register against an empty directory so the initial full scan indexes
        // nothing — docs/a.md is written only afterward, so the store has no
        // prior row for it when index_file_incremental first sees it.
        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        write(&dir, "docs/a.md", "# A\nfirst");
        let abs = dir.join("docs/a.md");

        // A brand-new path (no prior stored content) counts as changed.
        assert!(
            engine.index_file_incremental(&project, &abs).unwrap(),
            "first index of a new path must report changed"
        );

        // Re-indexing identical content reports no change.
        assert!(
            !engine.index_file_incremental(&project, &abs).unwrap(),
            "byte-identical reindex must report not-changed"
        );

        // Editing the content reports changed again.
        std::fs::write(&abs, "# A\nsecond").unwrap();
        assert!(
            engine.index_file_incremental(&project, &abs).unwrap(),
            "differing content must report changed"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn asset_path_enforces_allowlist_exclude_patterns_and_traversal_guard() {
        let dir = std::env::temp_dir().join(format!("waggledance-asset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        write(&dir, "readme.md", "# root");
        write(&dir, "images/logo.png", "fake-png-bytes");
        write(&dir, "images/secret.env", "SECRET=1");
        write(&dir, "images/LOGO.PNG", "fake-png-bytes-upper");
        write(&dir, "node_modules/pkg/logo.png", "vendored-png-bytes");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();

        // allowed extension → Ok
        assert!(engine.asset_path(&project.id, "images/logo.png").is_ok());

        // uppercase extension → Ok (case-insensitive)
        assert!(engine.asset_path(&project.id, "images/LOGO.PNG").is_ok());

        // disallowed extension → Err
        assert!(engine.asset_path(&project.id, "images/secret.env").is_err());

        // allowed extension but inside an excluded directory → Err
        assert!(engine
            .asset_path(&project.id, "node_modules/pkg/logo.png")
            .is_err());

        // traversal escape → Err, unchanged
        assert!(engine
            .asset_path(&project.id, "../../../../../../../etc/passwd")
            .is_err());

        #[cfg(unix)]
        {
            // A symlink named with an allowed extension but pointing at a
            // disallowed-extension target must still be rejected: the
            // extension check runs on the canonicalized (resolved) path,
            // not the pre-resolution symlink name.
            let target = dir.join("images/secret.env");
            let link = dir.join("images/bypass.png");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(engine.asset_path(&project.id, "images/bypass.png").is_err());

            // The highest-value vector: a symlink with an *allowed* extension
            // pointing at a readable file *outside* the project root. Its
            // extension passes, so only the containment guard (starts_with on
            // the canonical path) rejects it — lock that in.
            let outside = std::env::temp_dir()
                .join(format!("waggledance-outside-{}.png", std::process::id()));
            std::fs::write(&outside, "out-of-root-bytes").unwrap();
            let esc_link = dir.join("images/escape.png");
            std::os::unix::fs::symlink(&outside, &esc_link).unwrap();
            assert!(engine.asset_path(&project.id, "images/escape.png").is_err());
            std::fs::remove_file(&outside).ok();
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- refresh_stale matrix (D4) ----

    #[test]
    fn refresh_stale_reindexes_a_modified_file() {
        let dir =
            std::env::temp_dir().join(format!("waggledance-stale-mod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\noriginal body");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        assert_eq!(
            engine.store.file_content(&project.id, "docs/a.md").unwrap(),
            Some("# A\noriginal body".to_string())
        );

        // A different length changes size_bytes even at the same wall-clock
        // second, so the test never depends on mtime resolution.
        std::fs::write(dir.join("docs/a.md"), "# A\nchanged body, much longer now").unwrap();

        let n = engine.refresh_stale(&project.id).unwrap();
        assert_eq!(n, 1, "the one modified file must be re-indexed");
        assert_eq!(
            engine.store.file_content(&project.id, "docs/a.md").unwrap(),
            Some("# A\nchanged body, much longer now".to_string())
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_stale_leaves_an_untouched_file_content_unread() {
        let dir = std::env::temp_dir().join(format!(
            "waggledance-stale-untouched-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\nbody");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        let stored = engine
            .store
            .get_file(&project.id, "docs/a.md")
            .unwrap()
            .unwrap();

        // Overwrite the stored *content* with a marker while keeping the
        // stored size_bytes/modified_at exactly as they were — simulating
        // "index says this file's fs stat is X" without touching the file on
        // disk. If refresh_stale re-reads this file despite the metadata
        // matching, the marker is clobbered back to the real disk content;
        // if it correctly skips the content read, the marker survives.
        engine
            .store
            .upsert_file(&stored, "MARKER_UNTOUCHED_NOT_REREAD")
            .unwrap();

        let n = engine.refresh_stale(&project.id).unwrap();
        assert_eq!(n, 0, "an untouched file must not count as re-indexed");
        assert_eq!(
            engine.store.file_content(&project.id, "docs/a.md").unwrap(),
            Some("MARKER_UNTOUCHED_NOT_REREAD".to_string()),
            "content must not have been re-read from disk"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_stale_indexes_a_new_file() {
        let dir =
            std::env::temp_dir().join(format!("waggledance-stale-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\nbody");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        assert_eq!(engine.file_count(&project.id).unwrap(), 1);

        write(&dir, "docs/b.md", "# B\nbrand new file");
        let n = engine.refresh_stale(&project.id).unwrap();
        assert_eq!(n, 1, "the one new file must be indexed");
        assert_eq!(engine.file_count(&project.id).unwrap(), 2);
        assert!(engine
            .store
            .get_file(&project.id, "docs/b.md")
            .unwrap()
            .is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_stale_removes_a_deleted_file() {
        let dir =
            std::env::temp_dir().join(format!("waggledance-stale-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\nbody");
        write(&dir, "docs/b.md", "# B\nbody");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        assert_eq!(engine.file_count(&project.id).unwrap(), 2);

        std::fs::remove_file(dir.join("docs/b.md")).unwrap();
        engine.refresh_stale(&project.id).unwrap();

        assert_eq!(engine.file_count(&project.id).unwrap(), 1);
        assert!(engine
            .store
            .get_file(&project.id, "docs/b.md")
            .unwrap()
            .is_none());
        assert!(engine
            .store
            .get_file(&project.id, "docs/a.md")
            .unwrap()
            .is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_stale_keeps_a_gitignored_indexed_file() {
        let dir = std::env::temp_dir().join(format!(
            "waggledance-stale-gitignore-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\nbody");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        assert_eq!(engine.file_count(&project.id).unwrap(), 1);

        // A file under an excluded dir ("target" — Config::default's
        // exclude_patterns): scan_markdown_files (the walk) will never see
        // it, mirroring a .gitignore'd file. But view_file's
        // index_file_incremental has no such filter, so a file like this can
        // land in the index anyway — index it the same way IndexService
        // does directly (bypassing the walk), the way view_file would.
        write(&dir, "target/notes.md", "# Notes\nkept alive");
        let abs = dir.join("target/notes.md");
        IndexService::index_file(&engine.store, &project, &abs, engine.max_bytes())
            .unwrap()
            .expect("an excluded-but-readable file must still index directly");
        assert_eq!(engine.file_count(&project.id).unwrap(), 2);

        let n = engine.refresh_stale(&project.id).unwrap();
        assert_eq!(n, 0, "the excluded file is untouched, not re-read");
        assert_eq!(
            engine.file_count(&project.id).unwrap(),
            2,
            "walk-absence alone must never delete a row"
        );
        assert!(
            engine
                .store
                .get_file(&project.id, "target/notes.md")
                .unwrap()
                .is_some(),
            "gitignored/excluded-but-indexed file must survive refresh_stale"
        );
        assert_eq!(
            engine
                .store
                .file_content(&project.id, "target/notes.md")
                .unwrap(),
            Some("# Notes\nkept alive".to_string()),
            "the row's content/searchability must remain intact"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn refresh_stale_keeps_a_row_when_stat_is_permission_denied() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("waggledance-stale-denied-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\nbody");
        write(&dir, "locked/b.md", "# B\nbody");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        assert_eq!(engine.file_count(&project.id).unwrap(), 2);

        let locked_dir = dir.join("locked");
        // Strip the parent dir's permissions so stat-ing the child fails
        // with PermissionDenied, not NotFound — the row must be kept.
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = engine.refresh_stale(&project.id);

        // Restore permissions before any assertion (which could panic and
        // skip cleanup) so a locked-down temp dir is never left behind.
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        result.unwrap();
        assert_eq!(
            engine.file_count(&project.id).unwrap(),
            2,
            "a stat failure other than NotFound must never delete the row"
        );
        assert!(
            engine
                .store
                .get_file(&project.id, "locked/b.md")
                .unwrap()
                .is_some(),
            "permission-denied stat must keep the row"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn refresh_stale_leaves_the_index_untouched_when_the_root_vanishes() {
        let dir =
            std::env::temp_dir().join(format!("waggledance-stale-vanish-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, "docs/a.md", "# A\nbody");
        write(&dir, "docs/b.md", "# B\nbody");

        let engine = Engine::new(SqliteStore::open_in_memory().unwrap(), Config::default());
        let project = engine.register(&dir, None).unwrap();
        assert_eq!(engine.file_count(&project.id).unwrap(), 2);

        // The whole root disappears (unmounted volume, deleted checkout).
        std::fs::remove_dir_all(&dir).unwrap();

        let n = engine.refresh_stale(&project.id).unwrap();
        assert_eq!(n, 0);
        assert_eq!(
            engine.file_count(&project.id).unwrap(),
            2,
            "a vanished root must never empty the index"
        );
    }
}
