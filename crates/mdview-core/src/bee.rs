//! Pure reader for a project's `.bee/` store — bee-cockpit Slice 1.
//!
//! Turns `<root>/.bee/` into a typed [`BeeSnapshot`]. This module is
//! deliberately framework-free (no axum/tokio/hyper) so it stays inside
//! `mdview-core`, per the crate split documented at the top of `lib.rs`.
//!
//! Decisions honored here (see `docs/history/bee-cockpit/CONTEXT.md`):
//! - **D3** — presence is `<root>/.bee/` existing; absence is reported, not
//!   an error.
//! - **D4** — strictly read-only: every path here is opened for reading
//!   only, nothing is ever written.
//! - **D7** — cells sort into four buckets (Doing/Waiting/Stuck/Done);
//!   `dropped` and any unrecognized status land in none of them and are
//!   excluded from every count.
//! - **D8** — `active` is true iff at least one cell is `open` or `claimed`.
//! - **D9** — only live `.bee/cells/*.json` is read; `.bee/cells/archive/`
//!   and `.bee/logs/` are never opened.
//!
//! Every path-shaped value that crosses into a public field is rendered
//! relative to the project root (or reduced to a bare filename when it
//! falls outside the root) — no absolute path may survive into a
//! [`BeeSnapshot`]'s public fields. Malformed JSON degrades to a partial
//! snapshot with a note in [`BeeSnapshot::read_errors`] instead of
//! propagating an error that would take down a page render.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// One live cell, trimmed to what the cockpit board needs. Any path-shaped
/// field (`files`, `worker`) is relativized against the project root before
/// it reaches this struct — see [`relativize`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeCell {
    pub id: String,
    pub feature: String,
    pub title: String,
    pub lane: String,
    /// Raw status string as read from the cell file (`open`, `claimed`,
    /// `blocked`, `capped`, `dropped`, or anything else a future bee
    /// version introduces). Bucketing is derived from this, never stored
    /// redundantly.
    pub status: String,
    pub tier: Option<String>,
    /// Relative to the project root; never absolute.
    pub files: Vec<String>,
    /// `trace.worker`, relativized if it happens to be path-shaped.
    pub worker: Option<String>,
    pub claimed_at: Option<String>,
    pub capped_at: Option<String>,
}

/// The four D7 buckets. A `dropped` cell or one with an unrecognized status
/// lands in none of these and is excluded from every count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeBuckets {
    /// `status == "claimed"`.
    pub doing: Vec<BeeCell>,
    /// `status == "open"`.
    pub waiting: Vec<BeeCell>,
    /// `status == "blocked"`.
    pub stuck: Vec<BeeCell>,
    /// `status == "capped"`.
    pub done: Vec<BeeCell>,
}

/// The subset of `.bee/state.json` the cockpit shows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BeeState {
    pub phase: Option<String>,
    pub feature: Option<String>,
    pub mode: Option<String>,
}

/// A typed snapshot of a project's `.bee/` store at read time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeeSnapshot {
    /// True when `<root>/.bee/` exists (D3).
    pub present: bool,
    /// `None` when `.bee/state.json` is absent or malformed; see
    /// `read_errors` for why.
    pub state: Option<BeeState>,
    pub buckets: BeeBuckets,
    /// True when at least one cell is `open` or `claimed` (D8).
    pub active: bool,
    /// Human-readable notes naming what could not be read. Every path
    /// mentioned here is relative to the project root.
    pub read_errors: Vec<String>,
}

impl BeeSnapshot {
    /// The snapshot for a project whose root has no `.bee/` directory (D3).
    pub fn absent() -> Self {
        BeeSnapshot {
            present: false,
            state: None,
            buckets: BeeBuckets::default(),
            active: false,
            read_errors: Vec::new(),
        }
    }
}

/// Read `<root>/.bee/` into a typed [`BeeSnapshot`].
///
/// Pure and infallible: this function only opens files for reading, never
/// writes anything (D4), and never panics or returns `Err` — a missing or
/// malformed file is recorded in [`BeeSnapshot::read_errors`] and the read
/// continues with whatever else could be parsed.
pub fn read_snapshot(root: &Path) -> BeeSnapshot {
    let bee_dir = root.join(".bee");
    if !bee_dir.is_dir() {
        return BeeSnapshot::absent();
    }

    let mut read_errors = Vec::new();

    let state = read_state(&bee_dir, root, &mut read_errors);

    let mut buckets = BeeBuckets::default();
    let mut active = false;

    let cells_dir = bee_dir.join("cells");
    if cells_dir.is_dir() {
        let mut entries: Vec<PathBuf> = match fs::read_dir(&cells_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                // .bee/cells/archive/ (D9) has no .json extension of its own
                // and is filtered out here; the is_file() guard below is a
                // second, explicit line of defense against ever descending
                // into it.
                .filter(|p| p.is_file())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
                .collect(),
            Err(e) => {
                read_errors.push(format!(".bee/cells: could not list ({e})"));
                Vec::new()
            }
        };
        entries.sort();

        for path in entries {
            match parse_cell(&path, root) {
                Ok(cell) => {
                    let is_active = matches!(cell.status.as_str(), "open" | "claimed");
                    if is_active {
                        active = true;
                    }
                    match cell.status.as_str() {
                        "claimed" => buckets.doing.push(cell),
                        "open" => buckets.waiting.push(cell),
                        "blocked" => buckets.stuck.push(cell),
                        "capped" => buckets.done.push(cell),
                        // "dropped" and any unrecognized status: no bucket,
                        // no count (D7), read still succeeds.
                        _ => {}
                    }
                }
                Err(e) => {
                    read_errors.push(format!("{}: {e}", rel_str(&path, root)));
                }
            }
        }
    }

    BeeSnapshot {
        present: true,
        state,
        buckets,
        active,
        read_errors,
    }
}

fn read_state(bee_dir: &Path, root: &Path, read_errors: &mut Vec<String>) -> Option<BeeState> {
    let path = bee_dir.join("state.json");
    if !path.is_file() {
        // No state.json is a normal, expected shape (not every .bee/ has
        // reached a phase yet) — silent, not a read error.
        return None;
    }
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            read_errors.push(format!("{}: could not read ({e})", rel_str(&path, root)));
            return None;
        }
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(v) => Some(BeeState {
            phase: v.get("phase").and_then(Value::as_str).map(String::from),
            feature: v.get("feature").and_then(Value::as_str).map(String::from),
            mode: v.get("mode").and_then(Value::as_str).map(String::from),
        }),
        Err(e) => {
            read_errors.push(format!("{}: could not parse ({e})", rel_str(&path, root)));
            None
        }
    }
}

/// Parse one `.bee/cells/<id>.json` file into a [`BeeCell`], relativizing
/// every path-shaped value it carries.
fn parse_cell(path: &Path, root: &Path) -> Result<BeeCell, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("could not read ({e})"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!("could not parse ({e})"))?;

    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or("missing \"id\"")?
        .to_string();
    let status = v
        .get("status")
        .and_then(Value::as_str)
        .ok_or("missing \"status\"")?
        .to_string();
    let feature = v
        .get("feature")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let lane = v
        .get("lane")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let tier = v.get("tier").and_then(Value::as_str).map(String::from);

    let files = v
        .get("files")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| relativize(s, root))
                .collect()
        })
        .unwrap_or_default();

    let trace = v.get("trace");
    let worker = trace
        .and_then(|t| t.get("worker"))
        .and_then(Value::as_str)
        .map(|s| relativize(s, root));
    let claimed_at = trace
        .and_then(|t| t.get("claimed_at"))
        .and_then(Value::as_str)
        .map(String::from);
    let capped_at = trace
        .and_then(|t| t.get("capped_at"))
        .and_then(Value::as_str)
        .map(String::from);

    Ok(BeeCell {
        id,
        feature,
        title,
        lane,
        status,
        tier,
        files,
        worker,
        claimed_at,
        capped_at,
    })
}

/// Render `s` relative to `root` when it names a path under `root`. When `s`
/// is not absolute it is returned unchanged (the common case — most
/// path-shaped fields, like `trace.worker`, are plain identifiers, not
/// paths). When `s` is absolute but falls outside `root`, it is reduced to
/// its bare filename so no absolute prefix of any kind survives into a
/// public field.
fn relativize(s: &str, root: &Path) -> String {
    let p = Path::new(s);
    if !p.is_absolute() {
        return s.to_string();
    }
    match p.strip_prefix(root) {
        Ok(rel) => to_forward_slashes(rel),
        Err(_) => p
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(absolute path redacted)".to_string()),
    }
}

/// Render a path known to be a descendant of `root` relative to it.
fn rel_str(path: &Path, root: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) => to_forward_slashes(rel),
        Err(_) => path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(absolute path redacted)".to_string()),
    }
}

fn to_forward_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn cell_json(id: &str, status: &str) -> String {
        format!(
            r#"{{
                "id": "{id}",
                "feature": "demo",
                "lane": "standard",
                "title": "Cell {id}",
                "action": "do the thing",
                "verify": "cargo test",
                "files": [],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {{}},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "{status}",
                "tier": "generation",
                "trace": {{"worker": "w1"}}
            }}"#
        )
    }

    fn fresh_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mdview-bee-{name}-{}-{}",
            std::process::id(),
            name.len() // cheap per-name salt, keeps directories distinct across test fns
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Recursively collect (relative path, content bytes) for everything
    /// under `dir`, for the D4 read-only probe.
    fn snapshot_tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in std::fs::read_dir(cur).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                    let content = std::fs::read(&path).unwrap();
                    out.push((rel, content));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn buckets_all_five_statuses_dropped_absent() {
        let root = fresh_root("all-statuses");
        write(&root, ".bee/state.json", r#"{"phase":"swarming","feature":"demo","mode":"standard"}"#);
        write(&root, ".bee/cells/c-open.json", &cell_json("c-open", "open"));
        write(&root, ".bee/cells/c-claimed.json", &cell_json("c-claimed", "claimed"));
        write(&root, ".bee/cells/c-blocked.json", &cell_json("c-blocked", "blocked"));
        write(&root, ".bee/cells/c-capped.json", &cell_json("c-capped", "capped"));
        write(&root, ".bee/cells/c-dropped.json", &cell_json("c-dropped", "dropped"));

        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert_eq!(snap.buckets.doing.len(), 1);
        assert_eq!(snap.buckets.waiting.len(), 1);
        assert_eq!(snap.buckets.stuck.len(), 1);
        assert_eq!(snap.buckets.done.len(), 1);
        assert_eq!(snap.state.as_ref().unwrap().phase.as_deref(), Some("swarming"));

        let all_ids: Vec<&str> = snap
            .buckets
            .doing
            .iter()
            .chain(&snap.buckets.waiting)
            .chain(&snap.buckets.stuck)
            .chain(&snap.buckets.done)
            .map(|c| c.id.as_str())
            .collect();
        assert!(!all_ids.contains(&"c-dropped"), "dropped cell leaked into a bucket: {all_ids:?}");
        assert_eq!(all_ids.len(), 4);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn active_true_with_open_or_claimed_false_otherwise() {
        let active_root = fresh_root("active-yes");
        write(&active_root, ".bee/cells/a.json", &cell_json("a", "open"));
        assert!(read_snapshot(&active_root).active);
        std::fs::remove_dir_all(&active_root).ok();

        let inactive_root = fresh_root("active-no");
        write(&inactive_root, ".bee/cells/a.json", &cell_json("a", "capped"));
        write(&inactive_root, ".bee/cells/b.json", &cell_json("b", "dropped"));
        assert!(!read_snapshot(&inactive_root).active);
        std::fs::remove_dir_all(&inactive_root).ok();
    }

    #[test]
    fn bee_dir_absent_is_reported_not_error() {
        let root = fresh_root("no-bee");
        // no .bee/ created at all
        let snap = read_snapshot(&root);
        assert!(!snap.present);
        assert!(!snap.active);
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert_eq!(snap.buckets.stuck.len(), 0);
        assert_eq!(snap.buckets.done.len(), 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_cells_dir_yields_four_zero_buckets() {
        let root = fresh_root("empty-cells");
        std::fs::create_dir_all(root.join(".bee/cells")).unwrap();
        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert_eq!(snap.buckets.stuck.len(), 0);
        assert_eq!(snap.buckets.done.len(), 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_status_counted_nowhere_read_still_succeeds() {
        let root = fresh_root("unknown-status");
        write(&root, ".bee/cells/weird.json", &cell_json("weird", "quarantined"));
        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.read_errors.is_empty(), "unknown status should not be a read error: {:?}", snap.read_errors);
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert_eq!(snap.buckets.stuck.len(), 0);
        assert_eq!(snap.buckets.done.len(), 0);
        assert!(!snap.active);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn archived_cells_contribute_to_no_count() {
        let root = fresh_root("archive");
        write(&root, ".bee/cells/live.json", &cell_json("live", "capped"));
        write(
            &root,
            ".bee/cells/archive/demo/archived-1.json",
            &cell_json("archived-1", "capped"),
        );
        write(
            &root,
            ".bee/cells/archive/demo/archived-2.json",
            &cell_json("archived-2", "open"),
        );

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.done.len(), 1, "only the live capped cell should count");
        assert_eq!(snap.buckets.doing.len(), 0);
        assert_eq!(snap.buckets.waiting.len(), 0);
        assert!(!snap.active, "the archived open cell must not flip active");
        assert!(snap.buckets.done.iter().all(|c| c.id == "live"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn malformed_state_and_truncated_cell_degrade_to_partial_snapshot() {
        let root = fresh_root("malformed");
        write(&root, ".bee/state.json", "{ this is not valid json");
        write(&root, ".bee/cells/good.json", &cell_json("good", "open"));
        write(&root, ".bee/cells/bad.json", "{\"id\": \"bad\", \"status\": \"open\"");

        let snap = read_snapshot(&root);
        assert!(snap.present);
        assert!(snap.state.is_none());
        assert_eq!(snap.buckets.waiting.len(), 1, "the well-formed cell must still parse");
        assert_eq!(snap.buckets.waiting[0].id, "good");
        assert_eq!(snap.read_errors.len(), 2, "expected notes for state.json and bad.json: {:?}", snap.read_errors);
        assert!(snap.read_errors.iter().any(|e| e.contains("state.json")));
        assert!(snap.read_errors.iter().any(|e| e.contains("bad.json")));
        // every read_errors entry must itself be relative
        for e in &snap.read_errors {
            assert!(!e.contains(&root.to_string_lossy().into_owned()), "read_errors leaked the fixture root: {e}");
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_absolute_path_survives_into_public_fields() {
        let root = fresh_root("security");
        let root_str = root.to_string_lossy().into_owned();
        let outside_abs = std::env::temp_dir()
            .join("mdview-bee-outside-file.rs")
            .to_string_lossy()
            .into_owned();
        let inside_abs = root.join("src/inside.rs").to_string_lossy().into_owned();
        let worker_abs = root.join("workers/reader-1").to_string_lossy().into_owned();

        let body = format!(
            r#"{{
                "id": "leaky",
                "feature": "demo",
                "lane": "standard",
                "title": "Leaky cell",
                "action": "x",
                "verify": "x",
                "files": ["{}", "{}"],
                "read_first": [],
                "deps": [],
                "decisions": [],
                "must_haves": {{}},
                "behavior_change": false,
                "change_class": "behavior",
                "pbi": null,
                "status": "open",
                "tier": "generation",
                "trace": {{"worker": "{}"}}
            }}"#,
            inside_abs.replace('\\', "\\\\"),
            outside_abs.replace('\\', "\\\\"),
            worker_abs.replace('\\', "\\\\"),
        );
        write(&root, ".bee/cells/leaky.json", &body);

        let snap = read_snapshot(&root);
        assert_eq!(snap.buckets.waiting.len(), 1);
        let cell = &snap.buckets.waiting[0];

        for f in &cell.files {
            assert!(!Path::new(f).is_absolute(), "leaked absolute path in files[]: {f}");
            assert!(!f.contains(&root_str), "leaked fixture root in files[]: {f}");
        }
        let worker = cell.worker.as_deref().unwrap_or_default();
        assert!(!Path::new(worker).is_absolute(), "leaked absolute path in worker: {worker}");
        assert!(!worker.contains(&root_str), "leaked fixture root in worker: {worker}");

        // the in-root file must have relativized cleanly (not just filename-reduced)
        assert!(cell.files.iter().any(|f| f == "src/inside.rs"), "files: {:?}", cell.files);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reading_never_writes_the_bee_tree() {
        let root = fresh_root("read-only");
        write(&root, ".bee/state.json", r#"{"phase":"swarming"}"#);
        write(&root, ".bee/cells/a.json", &cell_json("a", "open"));
        write(&root, ".bee/cells/archive/demo/z.json", &cell_json("z", "capped"));

        let before = snapshot_tree(&root);
        let _ = read_snapshot(&root);
        let after = snapshot_tree(&root);

        assert_eq!(before, after, ".bee/ tree changed after a read");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_web_framework_dependency_declared() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
        for forbidden in ["axum", "tokio", "hyper"] {
            assert!(
                !manifest.lines().any(|l| l.trim_start().starts_with(forbidden)),
                "mdview-core/Cargo.toml must not depend on {forbidden}"
            );
        }
    }
}
