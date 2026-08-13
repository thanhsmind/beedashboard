//! Path-allowlist validation — the single ordered gate every agent-reachable
//! path passes through (ported from herdr-go's `security::paths`, itself
//! ported from airemote `internal/security/paths.go`). D2's per-project
//! containment (`docs/history/agent-terminal/CONTEXT.md`) is enforced with
//! this gate, constructed with the one project root a request is scoped to —
//! per P6, no second `allowed_roots` allowlist configuration surface comes
//! across; this module is generic infrastructure, not itself a config knob.
//!
//! The **ordering is the contract**, not an implementation detail:
//! 1. reject non-absolute
//! 2. reject raw traversal components (`.`/`..`)
//! 3. deny-list check on the *unresolved* path
//! 4. resolve all symlinks
//! 5. deny-list check again on the *resolved* path (catches a link planted
//!    inside an allowed root that points at denied territory)
//! 6. containment check **component-by-component** (never text-prefix — defeats
//!    the sibling-prefix trick `…/projects-evil` vs `…/projects`)
//! 7. deny by default
//!
//! Fail-closed throughout: any error, any ambiguity → refusal.

use std::path::{Component, Path, PathBuf};

/// Why a path was refused. Every variant is a hard denial — there is no
/// "allowed with a warning" state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathRefusal {
    NotAbsolute,
    TraversalComponent,
    DeniedSubtree(String),
    OutsideAllowedRoots,
    Unresolvable(String),
    /// Boundary construction itself is invalid (an allowed root is denied).
    InvalidBoundary(String),
}

impl std::fmt::Display for PathRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathRefusal::NotAbsolute => write!(f, "path is not absolute"),
            PathRefusal::TraversalComponent => write!(f, "path contains a traversal component"),
            PathRefusal::DeniedSubtree(p) => write!(f, "path is inside a denied subtree: {p}"),
            PathRefusal::OutsideAllowedRoots => write!(f, "path is outside every allowed root"),
            PathRefusal::Unresolvable(e) => write!(f, "path could not be resolved: {e}"),
            PathRefusal::InvalidBoundary(e) => write!(f, "invalid boundary configuration: {e}"),
        }
    }
}

impl std::error::Error for PathRefusal {}

/// A validated filesystem boundary: the set of allowed roots plus the fixed,
/// non-configurable hard-deny list. Construct once at startup; construction
/// **refuses** if any allowed root sits on or inside the deny list, or if the
/// allowed-roots set is empty (fail-closed — an empty allowlist is never
/// "allow everything").
#[derive(Debug, Clone)]
pub struct Boundary {
    allowed_roots: Vec<PathBuf>,
    denied: Vec<PathBuf>,
}

/// The fixed, non-configurable deny list. Self-flagged as known-incomplete:
/// signing keys, registry creds, and shell history are not yet here —
/// widening it is deliberately an operator decision, not a default.
fn hard_deny_list() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/etc"),
        PathBuf::from("/root"),
        PathBuf::from("/var/lib"),
        PathBuf::from("/boot"),
        PathBuf::from("/sys"),
        PathBuf::from("/proc"),
        PathBuf::from("/dev"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        for sub in [".ssh", ".aws", ".config", ".gnupg", ".kube", ".docker"] {
            v.push(home.join(sub));
        }
    }
    v
}

fn expand_with_canonical_forms(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut expanded = Vec::with_capacity(paths.len() * 2);
    for path in paths {
        if let Ok(canonical) = std::fs::canonicalize(&path) {
            if canonical != path {
                expanded.push(canonical);
            }
        }
        expanded.push(path);
    }
    expanded
}

impl Boundary {
    /// Build a boundary from operator-configured allowed roots. Fails closed:
    /// empty roots → error; any root inside the hard-deny list → error.
    pub fn new(allowed_roots: Vec<PathBuf>) -> Result<Self, PathRefusal> {
        if allowed_roots.is_empty() {
            return Err(PathRefusal::InvalidBoundary(
                "allowed-roots is empty (refusing to allow the whole filesystem)".into(),
            ));
        }
        let denied = expand_with_canonical_forms(hard_deny_list());
        // Each allowed root must be absolute and not on/inside a denied subtree.
        for root in &allowed_roots {
            if !root.is_absolute() {
                return Err(PathRefusal::InvalidBoundary(format!(
                    "allowed root is not absolute: {}",
                    root.display()
                )));
            }
            for d in &denied {
                if is_contained(root, d) || root == d {
                    return Err(PathRefusal::InvalidBoundary(format!(
                        "allowed root {} is inside denied subtree {}",
                        root.display(),
                        d.display()
                    )));
                }
            }
        }
        let allowed_roots = expand_with_canonical_forms(allowed_roots);
        Ok(Boundary {
            allowed_roots,
            denied,
        })
    }

    /// Validate an already-existing path through all 7 steps, returning the
    /// canonical (symlink-resolved) path on success.
    pub fn validate_existing(&self, input: &Path) -> Result<PathBuf, PathRefusal> {
        // Step 1: reject non-absolute.
        if !input.is_absolute() {
            return Err(PathRefusal::NotAbsolute);
        }
        // Step 2: reject raw traversal components.
        reject_traversal(input)?;
        // Step 3: deny-list on the unresolved path.
        self.check_denied(input)?;
        // Step 4: resolve all symlinks.
        let resolved =
            std::fs::canonicalize(input).map_err(|e| PathRefusal::Unresolvable(e.to_string()))?;
        // Step 5: deny-list again on the resolved path.
        self.check_denied(&resolved)?;
        // Step 6: component-wise containment in some allowed root.
        if !self.is_within_allowed(&resolved) {
            return Err(PathRefusal::OutsideAllowedRoots);
        }
        // Step 7: deny by default is the fallthrough — reaching here means allowed.
        Ok(resolved)
    }

    /// Validate a path's *intended* location without requiring it to exist yet
    /// (for a to-be-created directory). Resolves the nearest existing ancestor
    /// (so a planted symlink in the ancestor chain is still caught), then checks
    /// containment of the full intended path. The caller must still create with
    /// no-follow semantics (see `safe_create`).
    pub fn validate_intended(&self, input: &Path) -> Result<PathBuf, PathRefusal> {
        if !input.is_absolute() {
            return Err(PathRefusal::NotAbsolute);
        }
        reject_traversal(input)?;
        self.check_denied(input)?;
        // Resolve the nearest existing ancestor to defeat symlinked ancestors.
        let mut ancestor = input;
        let resolved_ancestor = loop {
            match std::fs::canonicalize(ancestor) {
                Ok(p) => break p,
                Err(_) => match ancestor.parent() {
                    Some(parent) => ancestor = parent,
                    None => {
                        return Err(PathRefusal::Unresolvable(
                            "no existing ancestor resolves".into(),
                        ))
                    }
                },
            }
        };
        self.check_denied(&resolved_ancestor)?;
        // Reconstruct the intended absolute path against the resolved ancestor
        // by appending the not-yet-existing tail components.
        let tail = input
            .strip_prefix(ancestor)
            .map_err(|e| PathRefusal::Unresolvable(e.to_string()))?;
        let intended = resolved_ancestor.join(tail);
        self.check_denied(&intended)?;
        if !self.is_within_allowed(&intended) {
            return Err(PathRefusal::OutsideAllowedRoots);
        }
        Ok(intended)
    }

    fn check_denied(&self, path: &Path) -> Result<(), PathRefusal> {
        for d in &self.denied {
            if path == d || is_contained(path, d) {
                return Err(PathRefusal::DeniedSubtree(d.display().to_string()));
            }
        }
        Ok(())
    }

    fn is_within_allowed(&self, path: &Path) -> bool {
        self.allowed_roots
            .iter()
            .any(|root| path == root || is_contained(path, root))
    }
}

/// Reject a path containing any `.` or `..` component. `RootDir`/`Normal` are
/// fine; a `ParentDir` or `CurDir` anywhere is a hard refusal.
fn reject_traversal(path: &Path) -> Result<(), PathRefusal> {
    for c in path.components() {
        match c {
            Component::ParentDir | Component::CurDir => {
                return Err(PathRefusal::TraversalComponent)
            }
            _ => {}
        }
    }
    Ok(())
}

/// D9b (`docs/history/projects-home/CONTEXT.md`): whether `path` is itself a
/// hard-deny-listed root, sits inside one, or **contains** one — containment
/// checked in both directions, not only the "is this candidate root inside a
/// denied root" half `Boundary::new`'s own `check_denied` answers for an
/// already-*allowed* root. A caller validating a single candidate root before
/// it ever becomes a `Boundary` (the register route) needs the other
/// direction too: `$HOME` is not itself on the deny list, but it *contains*
/// the deny-listed `$HOME/.ssh`, and `Boundary::new(vec![$HOME])` alone would
/// accept it — which is exactly how `POST path=$HOME` on the register route
/// slipped past the gate and went on to index (and later serve, unauthenticated)
/// markdown under `~/.ssh`, `~/.aws` and `~/.gnupg`. Reuses `hard_deny_list`
/// and its canonical-form expansion rather than exposing the list itself or
/// duplicating its contents in a caller crate. `path` is expected already
/// canonical (symlink-resolved); this makes no filesystem call of its own.
pub fn is_denied_root(path: &Path) -> bool {
    let denied = expand_with_canonical_forms(hard_deny_list());
    denied
        .iter()
        .any(|d| is_contained(path, d) || is_contained(d, path))
}

/// project-suggestions: is `candidate` (a pane's raw, unvalidated `cwd` —
/// exactly as herdr reported it, never passed through
/// `Boundary::validate_existing`) equal to or inside `root` (a registered
/// project's own, also-raw `root_path`)? Matched by path components exactly
/// like [`is_denied_root`] above matches the hard-deny list — never a text
/// prefix (`/root/projects-evil` is not contained in `/root/projects`),
/// reusing the same `is_contained` walk `Boundary` itself uses internally
/// rather than duplicating it.
///
/// Deliberately makes **no** filesystem call and does **no**
/// canonicalization. `Boundary::validate_existing`'s own
/// `std::fs::canonicalize` is exactly what refuses a pane whose directory
/// has since been deleted, whose reported `cwd` still carries a `.`/`..`
/// component, or whose registered project's root does not exist on disk
/// (`Boundary::new` never stats a root, so a since-deleted project root
/// still constructs one) — this predicate exists precisely to catch what
/// that call refuses to even look at, so a candidate this predicate accepts
/// must be dropped from a suggestion list in addition to, never instead of,
/// the existing `project_panes`-membership check.
pub fn is_contained_in_root(candidate: &Path, root: &Path) -> bool {
    is_contained(candidate, root)
}

/// Component-wise containment: is `path` equal to or inside `root`, matched by
/// path components (NOT a text prefix). `/a/projects-evil` is NOT contained in
/// `/a/projects` even though the string starts with it.
fn is_contained(path: &Path, root: &Path) -> bool {
    let mut pc = path.components();
    for rc in root.components() {
        match pc.next() {
            Some(c) if c == rc => continue,
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn empty_allowlist_is_refused() {
        assert!(matches!(
            Boundary::new(vec![]),
            Err(PathRefusal::InvalidBoundary(_))
        ));
    }

    #[test]
    fn allowed_root_inside_deny_list_is_refused() {
        // /etc is on the hard-deny list.
        let r = Boundary::new(vec![PathBuf::from("/etc/foo")]);
        assert!(matches!(r, Err(PathRefusal::InvalidBoundary(_))));
    }

    #[test]
    fn non_absolute_is_refused() {
        let dir = tmp();
        let b = Boundary::new(vec![dir.path().to_path_buf()]).unwrap();
        assert_eq!(
            b.validate_existing(Path::new("relative/path")),
            Err(PathRefusal::NotAbsolute)
        );
    }

    #[test]
    fn traversal_component_is_refused() {
        let dir = tmp();
        let b = Boundary::new(vec![dir.path().to_path_buf()]).unwrap();
        let evil = dir.path().join("..").join("escape");
        assert_eq!(
            b.validate_existing(&evil),
            Err(PathRefusal::TraversalComponent)
        );
    }

    #[test]
    fn path_inside_allowed_root_is_accepted() {
        let dir = tmp();
        let sub = dir.path().join("project");
        fs::create_dir(&sub).unwrap();
        let b = Boundary::new(vec![dir.path().to_path_buf()]).unwrap();
        let got = b.validate_existing(&sub).unwrap();
        assert_eq!(got, fs::canonicalize(&sub).unwrap());
    }

    #[test]
    fn sibling_prefix_trap_is_refused() {
        // `/root/projects-evil` must NOT be considered inside `/root/projects`.
        let dir = tmp();
        let allowed = dir.path().join("projects");
        let sibling = dir.path().join("projects-evil");
        fs::create_dir(&allowed).unwrap();
        fs::create_dir(&sibling).unwrap();
        let b = Boundary::new(vec![allowed.clone()]).unwrap();
        assert_eq!(
            b.validate_existing(&sibling),
            Err(PathRefusal::OutsideAllowedRoots)
        );
        // and the real child is allowed
        let child = allowed.join("a");
        fs::create_dir(&child).unwrap();
        assert!(b.validate_existing(&child).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_planted_in_allowed_root_pointing_outside_is_refused() {
        use std::os::unix::fs::symlink;
        let dir = tmp();
        let outside = tmp();
        let allowed = dir.path().join("projects");
        fs::create_dir(&allowed).unwrap();
        // link inside the allowed root that points at denied/outside territory
        let link = allowed.join("escape");
        symlink(outside.path(), &link).unwrap();
        let b = Boundary::new(vec![allowed.clone()]).unwrap();
        // canonicalize resolves the link to `outside`, which is outside allowed
        assert_eq!(
            b.validate_existing(&link),
            Err(PathRefusal::OutsideAllowedRoots)
        );
    }

    #[test]
    fn is_contained_is_component_wise() {
        assert!(is_contained(Path::new("/a/b/c"), Path::new("/a/b")));
        assert!(!is_contained(Path::new("/a/b-evil"), Path::new("/a/b")));
        assert!(is_contained(Path::new("/a/b"), Path::new("/a/b")));
    }

    // ── project-suggestions: is_contained_in_root, the raw (no-canonicalize)
    //    membership predicate a suggestion candidate is checked against ──────

    #[test]
    fn is_contained_in_root_true_when_candidate_is_inside_root() {
        assert!(is_contained_in_root(
            Path::new("/scratch/owned-project/sub"),
            Path::new("/scratch/owned-project")
        ));
    }

    #[test]
    fn is_contained_in_root_true_when_candidate_equals_root() {
        assert!(is_contained_in_root(
            Path::new("/scratch/owned-project"),
            Path::new("/scratch/owned-project")
        ));
    }

    #[test]
    fn is_contained_in_root_false_for_a_sibling_that_merely_shares_a_prefix() {
        // `/scratch/owned-project-evil` must NOT read as contained in
        // `/scratch/owned-project` even though the string starts with it.
        assert!(!is_contained_in_root(
            Path::new("/scratch/owned-project-evil"),
            Path::new("/scratch/owned-project")
        ));
    }

    #[test]
    fn is_contained_in_root_false_when_candidate_is_genuinely_outside() {
        assert!(!is_contained_in_root(
            Path::new("/scratch/stray-agent-cwd"),
            Path::new("/scratch/owned-project")
        ));
    }

    #[test]
    fn intended_path_not_yet_existing_is_accepted() {
        let dir = tmp();
        let b = Boundary::new(vec![dir.path().to_path_buf()]).unwrap();
        let intended = dir.path().join("new-project");
        assert!(b.validate_intended(&intended).is_ok());
    }

    // ── D9b: is_denied_root's containment in both directions ──────────────

    #[test]
    fn is_denied_root_true_when_path_equals_a_denied_root() {
        assert!(is_denied_root(Path::new("/etc")));
    }

    #[test]
    fn is_denied_root_true_when_path_is_inside_a_denied_root() {
        assert!(is_denied_root(Path::new("/etc/hosts")));
    }

    #[test]
    fn is_denied_root_true_when_path_contains_a_denied_root() {
        // "/" contains every hard-deny-listed root, /etc among them -- this
        // is the direction `Boundary::new` alone never checks.
        assert!(is_denied_root(Path::new("/")));
    }

    #[test]
    fn is_denied_root_true_when_home_contains_a_denied_subdirectory() {
        // `$HOME/.ssh` is on the deny list even when it doesn't exist on
        // disk -- hard_deny_list is a fixed set of paths, not a directory
        // listing -- so $HOME itself, which contains it, must be denied too.
        if let Some(home) = std::env::var_os("HOME") {
            assert!(is_denied_root(Path::new(&home)));
        }
    }

    #[test]
    fn is_denied_root_false_for_an_unrelated_path() {
        let dir = tmp();
        assert!(!is_denied_root(dir.path()));
    }
}
