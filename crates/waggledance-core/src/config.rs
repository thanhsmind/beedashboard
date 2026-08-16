//! Config (`~/.waggledance/config.toml`). Atomic write, resilient load (corrupt → default).
//! Mirrors PRD §10.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub mcp: McpConfig,
    pub indexing: IndexingConfig,
    pub renderer: RendererConfig,
    pub search: SearchConfig,
    pub terminal: TerminalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
    /// Optional display hostname. When set, rendered view URLs use this
    /// instead of `host`/the daemon's bind address; the bind/connect
    /// address itself is unaffected.
    #[serde(alias = "host_name")]
    pub hostname: Option<String>,
    pub open_browser_on_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub enabled: bool,
    pub transport: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexingConfig {
    pub debounce_ms: u64,
    pub max_file_size_mb: u64,
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RendererConfig {
    pub theme: String,
    pub syntax_highlight_theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub enable_fts: bool,
    pub enable_semantic: bool,
}

/// The D7 opt-in switches for the agent terminal surface, all off until the
/// user turns them on from the settings page. `#[derive(Default)]` gives
/// every switch `false`, matching a config that has never seen this
/// section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// The terminal surface itself (D2/D3) — panes and screens are reachable
    /// only once this is on.
    pub enabled: bool,
    /// D7: keep the herdr supervisor process alive. waggledance spawns nothing
    /// while this is off.
    pub supervisor_enabled: bool,
    /// D7: Telegram notification on agent status change. waggledance makes no
    /// outbound call while this is off.
    pub notify_enabled: bool,
    /// toa-4 (D9): the Unassigned group — panes that live outside every
    /// registered project's root. This group has no containment check of
    /// its own; before terminal-open-access removed the terminal's session,
    /// that session was the only thing authorizing it (`server.rs`'s
    /// `unassigned_panes` doc comment). With no session left to lean on,
    /// this switch is a second, deliberate gate on top of `enabled` above —
    /// both must be on for the group's routes to answer, so turning off
    /// `enabled` alone still closes this group, and turning this switch on
    /// while `enabled` is off opens nothing. `#[derive(Default)]` on this
    /// struct gives it `false`, matching a config that has never mentioned
    /// it — the same "off unless the owner made a deliberate act" rule
    /// every other switch in this section follows.
    pub unassigned_enabled: bool,
    /// D8/P4: operator-authored agent-create presets, keyed by label — the
    /// terminal page's creation controls
    /// (`crates/waggledance/src/views.rs::terminal_create_controls`) offer
    /// exactly these labels and nothing else, and
    /// `crates/waggledance/src/server.rs::terminal_create_agent` is the only
    /// place a label is ever turned into the argv it keys into. No HTTP
    /// request ever supplies or reads an `argv` — this config field is the
    /// only source. Defaults to empty: a fresh install refuses every
    /// preset-create request until the operator configures at least one.
    #[serde(default)]
    pub agent_presets: Vec<AgentPreset>,
    /// D7/D9's notification destination: the Telegram chat id status-change
    /// alerts are sent to. Unlike the bot token (never a `Config` field —
    /// see [`notify_credential_path_override`] below) a chat id names a
    /// destination, not a credential, so it is an ordinary `Config` field:
    /// visible on `GET /api/config` and in the settings HTML like any other
    /// setting. `None` (the default) means no destination is configured —
    /// `TelegramNotifier::new` (`crates/waggledance/src/notify/telegram.rs`)
    /// requires both halves, so a configuration missing this one never
    /// attempts a delivery no matter what the switch says.
    #[serde(default)]
    pub notify_chat_id: Option<String>,
}

/// One D8 agent-create preset: a label the terminal page's creation
/// controls offer, keyed to the argv that is started when a client picks it
/// by name. Operator-authored config only — never populated from, or
/// exposed raw to, an HTTP request body (`crates/waggledance/src/server.rs`'s
/// create routes deserialize only a `preset` label, never `argv`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentPreset {
    pub label: String,
    pub argv: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 7700,
            // Bind all interfaces by default so the viewer is reachable from
            // other devices on the LAN (and from a browser when the daemon runs
            // on a remote host). The server has no auth at all (terminal-open-access
            // D1 removed the agent terminal's own token gate, its last one) —
            // every route stays open; `serve()` prints a non-loopback exposure
            // warning at startup.
            host: "0.0.0.0".into(),
            hostname: None,
            open_browser_on_start: false,
        }
    }
}
impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            transport: "stdio".into(),
        }
    }
}
impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 200,
            max_file_size_mb: 10,
            exclude_patterns: vec![
                ".git".into(),
                "node_modules".into(),
                ".venv".into(),
                "target".into(),
                "dist".into(),
            ],
        }
    }
}
impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            syntax_highlight_theme: "github-dark".into(),
        }
    }
}
impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            enable_fts: true,
            enable_semantic: false,
        }
    }
}
/// `~/.waggledance/` — the app data directory (created on demand).
pub fn data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".waggledance")
}

/// The pre-rename data directory (`~/.mdview/`), read only by the D2
/// migration below.
fn legacy_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mdview")
}

/// D2: armed only by `crate::cli::run` in the `waggledance` binary crate —
/// the single dispatch point every real subcommand (`serve`, `open`,
/// `doctor`, `mcp`, …) passes through, and re-armed the same way in a
/// re-exec'd daemon process (`spawn_daemon_detached`). Defaults to
/// disarmed, so this crate's own unit tests *and* every downstream crate
/// that links `waggledance-core` as an ordinary dependency (`#[cfg(test)]`
/// here never activates for them) can call `resolve_data_dir`/`data_dir`
/// through any of dozens of route-level tests with no override and never
/// reach a real, unoverridden `~/.mdview` or `~/.waggledance` — the "opt
/// out that the test suite sets" this migration needs, expressed as an
/// opt-in only the real CLI entry point ever exercises, because auditing
/// every test call site for an opt-out would have been both impractical and
/// one missed call away from renaming a developer's real home directory.
static DATA_DIR_MIGRATION_ARMED: AtomicBool = AtomicBool::new(false);

/// Arms the D2 migration for the remainder of this process. See
/// [`DATA_DIR_MIGRATION_ARMED`].
pub fn arm_data_dir_migration() {
    DATA_DIR_MIGRATION_ARMED.store(true, Ordering::SeqCst);
}

static MIGRATE_DATA_DIR_ONCE: Once = Once::new();

/// D2: `~/.mdview` → `~/.waggledance`, attempted at most once per process,
/// only when armed (see [`DATA_DIR_MIGRATION_ARMED`]). A no-op when the new
/// directory already exists (never overwrite it, and never touch the old
/// one) or when the old directory does not exist (nothing to migrate).
fn migrate_data_dir_once() {
    if !DATA_DIR_MIGRATION_ARMED.load(Ordering::SeqCst) {
        return;
    }
    MIGRATE_DATA_DIR_ONCE.call_once(|| {
        let old_dir = legacy_data_dir();
        let new_dir = data_dir();
        if let Err(e) = migrate_data_dir(&old_dir, &new_dir) {
            tracing::warn!(
                "failed to migrate data directory from {} to {}: {e}",
                old_dir.display(),
                new_dir.display()
            );
        }
    });
}

/// The D2 migration's testable core: renames `old_dir` to `new_dir` exactly
/// when `new_dir` is absent and `old_dir` exists. Idempotent by
/// construction — a second call after the first has succeeded already sees
/// `old_dir` gone (or `new_dir` present) and returns `Ok(())` without
/// touching the filesystem again.
fn migrate_data_dir(old_dir: &Path, new_dir: &Path) -> std::io::Result<()> {
    if new_dir.exists() || !old_dir.exists() {
        return Ok(());
    }
    rename_data_dir(old_dir, new_dir)
}

/// Unconditionally attempts the rename and logs exactly one line naming
/// both paths on success. `cmd_open` (`crates/waggledance/src/cli.rs`)
/// resolves the data directory and then spawns the daemon as a *separate*
/// process, so two processes can race this same rename. `fs::rename` is
/// atomic on unix; the losing process's `old_dir` is already gone by the
/// time its own `rename` call runs, which surfaces as `NotFound` — treated
/// here as success, never an error, so an ordinary `open` that merely lost
/// the race never aborts because of it.
fn rename_data_dir(old_dir: &Path, new_dir: &Path) -> std::io::Result<()> {
    match std::fs::rename(old_dir, new_dir) {
        Ok(()) => {
            tracing::info!(
                "migrated data directory from {} to {}",
                old_dir.display(),
                new_dir.display()
            );
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `data_dir()`, or `override_dir` when given. Callers that must be testable
/// without touching the developer's real `~/.waggledance` (route handlers
/// exercised through a test harness) resolve the data directory through
/// this instead of calling `data_dir()` directly. With `override_dir` unset
/// this returns exactly what `data_dir()` returns, after giving the D2
/// migration (armed only in the real CLI process) its one chance per
/// process to run.
pub fn resolve_data_dir(override_dir: Option<&Path>) -> PathBuf {
    match override_dir {
        Some(dir) => dir.to_path_buf(),
        None => {
            migrate_data_dir_once();
            data_dir()
        }
    }
}

/// Routes through [`resolve_data_dir`] (not `data_dir()` directly) so every
/// caller — `doctor`, `build_engine()`, the MCP server, `cmd_config_edit`,
/// `serve` — gets the D2 migration's one chance to run before it ever
/// creates `~/.waggledance/` itself, without each of those call sites
/// needing its own migration call.
pub fn config_path() -> PathBuf {
    resolve_data_dir(None).join("config.toml")
}

/// `config_path()`, or `override_dir/config.toml` when given.
pub fn config_path_override(override_dir: Option<&Path>) -> PathBuf {
    resolve_data_dir(override_dir).join("config.toml")
}

/// See [`config_path`]'s doc comment: routes through [`resolve_data_dir`]
/// for the same reason.
pub fn registry_db_path() -> PathBuf {
    resolve_data_dir(None).join("registry.db")
}

/// See [`config_path`]'s doc comment: routes through [`resolve_data_dir`]
/// for the same reason.
pub fn daemon_lock_path() -> PathBuf {
    resolve_data_dir(None).join("daemon.lock")
}

/// `<data_dir>/notify.sqlite`, or `override_dir/notify.sqlite` when given —
/// the D7/D9 notification outbox (`crate::notify_store::NotifyStore`),
/// mirroring `config_path_override` so a route-level test never touches the
/// real `~/.waggledance`.
pub fn notify_store_path_override(override_dir: Option<&Path>) -> PathBuf {
    resolve_data_dir(override_dir).join("notify.sqlite")
}

/// File name for the Telegram bot token, written beside `config.toml` in the
/// same data directory (P1's rule, extended to this second secret): `Config`
/// is serialized whole and unauthenticated by `GET /api/config`, so a
/// credential stored inside it would be one request away regardless of what
/// the settings HTML masks. A distinct file from the agent terminal's own
/// (now-unused) `terminal.token` — saving one never disturbs the other.
const NOTIFY_CREDENTIAL_FILE_NAME: &str = "telegram.token";

/// `<data_dir>/telegram.token`, or `override_dir/telegram.token` when given
/// — mirrors `config_path_override` so a route-level test never touches the
/// real `~/.waggledance`.
pub fn notify_credential_path_override(override_dir: Option<&Path>) -> PathBuf {
    resolve_data_dir(override_dir).join(NOTIFY_CREDENTIAL_FILE_NAME)
}

/// Persist the Telegram bot token beside the config: a fresh `O_EXCL`-created
/// temp file (never collides with, or inherits the permissions of, anything
/// already there), owner-only permissions where the platform supports them
/// (unix `0600`), then `rename`d over the target — so the target path is
/// always either the previous complete file or the new one, never a partial
/// write, and a failed write never touches it at all. Lives in
/// `waggledance-core` rather than the `waggledance` binary crate because both the
/// settings route (`crates/waggledance/src/server.rs`) and the notify reconciler
/// (`crates/waggledance/src/main.rs`) must reach it.
///
/// The temp file's name carries fresh randomness (agent-terminal-21) rather
/// than `std::process::id()` alone. A fixed, pid-only name is stable for the
/// life of the process, so `create_new` makes the *second* call in that
/// process (or a call racing another thread's save, or a call after a
/// previous crash left its temp file behind before the rename landed)
/// collide on the exact same path and fail permanently, silently, while a
/// caller that discards the `Result` (as
/// `crates/waggledance/src/server.rs::update_terminal_config` does today) goes
/// on to report success anyway. A fresh random suffix on every call makes
/// that collision astronomically unlikely instead.
///
/// A failure here is logged (`tracing::warn!`) rather than only returned —
/// today's one caller discards the `Result` with `let _ =`, so this is what
/// actually surfaces the failure until that caller stops doing so; a caller
/// that does check the `Result` still gets the real error either way.
pub fn save_notify_credential(path: &Path, secret: &str) -> Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| Error::Config("credential path has no parent directory".into()))?;
    std::fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!(
        "{NOTIFY_CREDENTIAL_FILE_NAME}.tmp-{}",
        random_temp_suffix()
    ));
    let result = write_owner_only(&tmp_path, secret.as_bytes())
        .and_then(|()| std::fs::rename(&tmp_path, path).map_err(Error::from));
    if let Err(ref e) = result {
        tracing::warn!(
            "failed to save notify credential to {}: {e}",
            path.display()
        );
        // Best-effort: a failed write never leaves debris beside the
        // credential for a later save to trip over.
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

/// 8 hex characters of fresh randomness, generated directly since this call
/// has no token of its own to slice a suffix from.
fn random_temp_suffix() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(unix)]
fn write_owner_only(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn write_owner_only(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// The credential currently on disk, or `None` if it has never been saved.
/// Returns the raw secret — callers (the notify reconciler in the `waggledance`
/// binary crate) may use it only to build an outbound client, never to
/// answer an HTTP response. Use [`masked_notify_credential`] for anything
/// rendered to a client.
pub fn load_notify_credential(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The credential's last four characters, or `None` if it has never been
/// saved — the only view of the credential any HTTP response may ever
/// carry. Unlike the terminal token's reveal-once (P2), there is no `Full`
/// view here at all: the form that sets this value is write-only, so this
/// function is called on every render, including the one immediately after
/// a save.
pub fn masked_notify_credential(path: &Path) -> Option<String> {
    load_notify_credential(path).map(|s| mask_secret(&s))
}

fn mask_secret(secret: &str) -> String {
    let n = secret.chars().count();
    if n <= 4 {
        "*".repeat(n)
    } else {
        let visible: String = secret.chars().skip(n - 4).collect();
        format!("{}{}", "*".repeat(n - 4), visible)
    }
}

impl Config {
    /// Load config; a missing or corrupt file resolves to defaults (never panics).
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("config parse failed ({e}); using defaults");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    /// Atomic write: serialize → temp file → rename (survives crash mid-write).
    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text =
            toml::to_string_pretty(self).map_err(|e| Error::Config(format!("serialize: {e}")))?;
        write_atomic(path, text.as_bytes())
    }
}

/// Atomic file write via temp-in-same-dir + rename. Shared by config & registry snapshots.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".{}.tmp{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("f"),
        std::process::id()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_config_falls_back_to_default() {
        let dir = std::env::temp_dir().join(format!("waggledance-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "this is not = valid : toml ][").unwrap();
        let c = Config::load_from(&p);
        assert_eq!(c.server.port, 7700);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn roundtrip_atomic_save_load() {
        let dir = std::env::temp_dir().join(format!("waggledance-cfg2-{}", std::process::id()));
        let p = dir.join("config.toml");
        let mut c = Config::default();
        c.server.port = 9999;
        c.save_to(&p).unwrap();
        let loaded = Config::load_from(&p);
        assert_eq!(loaded.server.port, 9999);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_data_dir_uses_override_when_set() {
        let dir =
            std::env::temp_dir().join(format!("waggledance-cfg-override-{}", std::process::id()));
        assert_eq!(resolve_data_dir(Some(&dir)), dir);
        assert_eq!(config_path_override(Some(&dir)), dir.join("config.toml"));
    }

    #[test]
    fn resolve_data_dir_falls_back_to_data_dir_when_unset() {
        // Safe by construction, not by any per-test action: `resolve_data_dir(None)`
        // does call the D2 migration's gate on the way to `data_dir()`, but that gate
        // is disarmed (`DATA_DIR_MIGRATION_ARMED` defaults `false`) everywhere except
        // inside `crate::cli::run`, which nothing in this test binary ever calls — so
        // this assertion can compare against the developer's real, unoverridden
        // `~/.waggledance` without ever renaming it.
        assert!(!DATA_DIR_MIGRATION_ARMED.load(Ordering::SeqCst));
        assert_eq!(resolve_data_dir(None), data_dir());
        assert_eq!(config_path_override(None), config_path());
    }

    /// D2's required proof, from the top: the suite can never touch the real
    /// home directory. `data_dir()`/`config_path()`/`registry_db_path()`/
    /// `daemon_lock_path()`/`resolve_data_dir(None)` all route through the
    /// same migration gate, and that gate reads disarmed here — the same
    /// invariant the previous test leans on, asserted on its own so a
    /// regression in *this* fact fails loudly instead of only as a side
    /// effect of an unrelated assertion.
    #[test]
    fn migration_is_disarmed_by_default_so_the_suite_never_touches_real_home() {
        assert!(!DATA_DIR_MIGRATION_ARMED.load(Ordering::SeqCst));
    }

    /// A fresh scratch pair of directories under `std::env::temp_dir()` —
    /// every migration test below uses this instead of the real
    /// `dirs::home_dir()`-derived paths, and exercises `migrate_data_dir`
    /// directly rather than through `resolve_data_dir`, so none of them
    /// depend on (or risk) the migration ever being armed.
    fn scratch_migration_dirs(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "waggledance-migrate-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let old_dir = base.join("old-data-dir");
        let new_dir = base.join("new-waggledance");
        std::fs::create_dir_all(&base).unwrap();
        (base, old_dir, new_dir)
    }

    #[test]
    fn migrates_once_and_registry_db_survives_with_its_rows_intact() {
        let (base, old_dir, new_dir) = scratch_migration_dirs("basic");
        std::fs::create_dir_all(&old_dir).unwrap();
        // Stands in for real row data: the migration must move bytes
        // unchanged, never open or reinterpret the sqlite file.
        let registry_bytes = b"sqlite-format-3\x00fake-rows-abc123";
        std::fs::write(old_dir.join("registry.db"), registry_bytes).unwrap();
        std::fs::write(old_dir.join("config.toml"), b"[server]\nport = 4242\n").unwrap();

        let result = migrate_data_dir(&old_dir, &new_dir);
        assert!(result.is_ok(), "migration must succeed: {result:?}");
        assert!(
            !old_dir.exists(),
            "old directory must be gone after migration"
        );
        assert!(new_dir.exists(), "new directory must exist after migration");
        assert_eq!(
            std::fs::read(new_dir.join("registry.db")).unwrap(),
            registry_bytes,
            "registry.db's rows must survive the migration unchanged"
        );
        assert_eq!(
            std::fs::read_to_string(new_dir.join("config.toml")).unwrap(),
            "[server]\nport = 4242\n"
        );

        // Idempotent: a second call (the "run once per process" guard's
        // effect, proved at the pure-function level) is a safe no-op.
        let second = migrate_data_dir(&old_dir, &new_dir);
        assert!(
            second.is_ok(),
            "a repeat migration call must stay Ok: {second:?}"
        );
        assert_eq!(
            std::fs::read(new_dir.join("registry.db")).unwrap(),
            registry_bytes,
            "a repeat call must never touch the already-migrated data"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn skipped_when_the_new_dir_already_exists() {
        let (base, old_dir, new_dir) = scratch_migration_dirs("new-exists");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::write(old_dir.join("registry.db"), b"old-untouched-data").unwrap();
        std::fs::create_dir_all(&new_dir).unwrap();
        std::fs::write(new_dir.join("registry.db"), b"already-migrated-data").unwrap();

        let result = migrate_data_dir(&old_dir, &new_dir);
        assert!(result.is_ok());
        assert!(
            old_dir.exists(),
            "the old directory must never be touched once the new one exists"
        );
        assert_eq!(
            std::fs::read(new_dir.join("registry.db")).unwrap(),
            b"already-migrated-data",
            "an existing new directory must never be overwritten"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn no_op_when_the_old_dir_is_absent() {
        let (base, old_dir, new_dir) = scratch_migration_dirs("old-absent");
        // old_dir deliberately never created.
        let result = migrate_data_dir(&old_dir, &new_dir);
        assert!(result.is_ok());
        assert!(
            !new_dir.exists(),
            "nothing to migrate must never create the new directory"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// The exact defect the concurrency note calls out: `cmd_open` resolves
    /// the data directory and then spawns the daemon as a separate process,
    /// so two processes can race this rename. The losing process's source
    /// directory is already gone by the time its own `fs::rename` call
    /// runs — `NotFound` — and that must read as success, never an error,
    /// or an ordinary `open` that merely lost the race would abort.
    #[test]
    fn the_losing_side_of_a_concurrent_rename_returns_success_not_an_error() {
        let (base, old_dir, new_dir) = scratch_migration_dirs("race-loser");
        // old_dir is never created here: this is exactly the state the
        // losing process's `old_dir.exists()` check would already see if it
        // ran after the winner's rename completed. Calling the unconditional
        // rename helper directly (bypassing that check) proves the
        // `fs::rename` failure itself — not just the guard in front of it —
        // is what turns "already gone" into `Ok(())`.
        let result = rename_data_dir(&old_dir, &new_dir);
        assert!(
            result.is_ok(),
            "a rename racing an already-vanished source must succeed, not error: {result:?}"
        );
        assert!(!new_dir.exists());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn default_host_binds_all_interfaces() {
        // Fresh installs must default to the LAN-reachable wildcard bind.
        assert_eq!(ServerConfig::default().host, "0.0.0.0");
    }

    #[test]
    fn terminal_switches_default_off_and_carry_no_token_field() {
        // A config that has never seen the terminal section — the shape a
        // pre-existing install's config.toml has today — must still resolve
        // every switch to off, never on by an absent-field accident.
        let c = Config::default();
        assert!(!c.terminal.enabled);
        assert!(!c.terminal.supervisor_enabled);
        assert!(!c.terminal.notify_enabled);
        assert!(!c.terminal.unassigned_enabled);

        // Round-trips through TOML with no token anywhere in the section.
        let dir =
            std::env::temp_dir().join(format!("waggledance-cfg-terminal-{}", std::process::id()));
        let p = dir.join("config.toml");
        c.save_to(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("[terminal]"));
        assert!(!text.to_lowercase().contains("token"));
        let loaded = Config::load_from(&p);
        assert!(!loaded.terminal.enabled);
        assert!(!loaded.terminal.supervisor_enabled);
        assert!(!loaded.terminal.notify_enabled);
        assert!(!loaded.terminal.unassigned_enabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unassigned_switch_defaults_off_when_the_key_is_absent_from_the_file() {
        // toa-4 (D9): the shipped-default test above only proves the
        // switch is off in a file *this build* wrote (which always
        // includes the key, since `#[serde(default)]` doesn't suppress
        // serialization). The truth that matters is stronger: an
        // *existing* install's config.toml — hand-authored or written by a
        // build that predates this switch entirely — has never heard of
        // `unassigned_enabled`, and loading it must still resolve to off,
        // not fail to parse and not silently read as on.
        let dir = std::env::temp_dir().join(format!(
            "waggledance-cfg-unassigned-absent-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(
            &p,
            "[terminal]\nenabled = true\nsupervisor_enabled = true\n",
        )
        .unwrap();
        let loaded = Config::load_from(&p);
        assert!(loaded.terminal.enabled);
        assert!(loaded.terminal.supervisor_enabled);
        assert!(!loaded.terminal.unassigned_enabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn agent_presets_default_to_empty_and_roundtrip() {
        // A fresh config (or one that has never seen D8's preset list) must
        // start empty — the terminal page's creation controls and the
        // preset-create route both fail closed on an empty list.
        let c = Config::default();
        assert!(c.terminal.agent_presets.is_empty());

        let dir =
            std::env::temp_dir().join(format!("waggledance-cfg-presets-{}", std::process::id()));
        let p = dir.join("config.toml");
        let mut c = Config::default();
        c.terminal.agent_presets = vec![AgentPreset {
            label: "Claude".into(),
            argv: vec!["claude".into()],
        }];
        c.save_to(&p).unwrap();
        let loaded = Config::load_from(&p);
        assert_eq!(loaded.terminal.agent_presets.len(), 1);
        assert_eq!(loaded.terminal.agent_presets[0].label, "Claude");
        assert_eq!(
            loaded.terminal.agent_presets[0].argv,
            vec!["claude".to_string()]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hostname_defaults_to_none_and_roundtrips_when_set() {
        assert_eq!(ServerConfig::default().hostname, None);

        let dir = std::env::temp_dir().join(format!("waggledance-cfg3-{}", std::process::id()));
        let p = dir.join("config.toml");
        let mut c = Config::default();
        c.server.hostname = Some("my-machine.local".into());
        c.save_to(&p).unwrap();
        let loaded = Config::load_from(&p);
        assert_eq!(loaded.server.hostname.as_deref(), Some("my-machine.local"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn notify_chat_id_defaults_to_none_and_roundtrips() {
        // Unlike the credential, the destination is an ordinary Config field
        // (it names a target, not a secret) — round-trips through TOML like
        // every other setting.
        assert_eq!(Config::default().terminal.notify_chat_id, None);

        let dir =
            std::env::temp_dir().join(format!("waggledance-cfg-chatid-{}", std::process::id()));
        let p = dir.join("config.toml");
        let mut c = Config::default();
        c.terminal.notify_chat_id = Some("-100123456789".into());
        c.save_to(&p).unwrap();
        let loaded = Config::load_from(&p);
        assert_eq!(
            loaded.terminal.notify_chat_id.as_deref(),
            Some("-100123456789")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn notify_credential_absent_until_saved_then_masked_never_full() {
        let dir = std::env::temp_dir().join(format!("waggledance-cfg-cred-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = notify_credential_path_override(Some(&dir));

        assert_eq!(load_notify_credential(&path), None);
        assert_eq!(masked_notify_credential(&path), None);

        let secret = "shhh-bot-token-abcd1234";
        save_notify_credential(&path, secret).unwrap();

        assert_eq!(load_notify_credential(&path).as_deref(), Some(secret));
        let masked = masked_notify_credential(&path).unwrap();
        assert_eq!(masked, "*******************1234");
        assert!(
            !masked.contains(secret),
            "masked view must never carry the full secret"
        );

        // Saving again (rotation) overwrites atomically rather than
        // appending or leaving a stale temp file behind.
        save_notify_credential(&path, "second-secret-9999").unwrap();
        assert_eq!(
            load_notify_credential(&path).as_deref(),
            Some("second-secret-9999")
        );
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec![NOTIFY_CREDENTIAL_FILE_NAME.to_string()],
            "no leftover temp file beside the credential: {entries:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn notify_credential_never_becomes_a_config_field() {
        // The leak assertion is written against the value that would leak
        // (the fixture's own generated secret and the file path it lives
        // beside), not a hardcoded literal — a decorative assertion would
        // stay green even if the credential quietly became a Config field
        // under a different name.
        let dir =
            std::env::temp_dir().join(format!("waggledance-cfg-cred-leak-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cred_path = notify_credential_path_override(Some(&dir));
        let secret = "definitely-a-secret-value-42";
        save_notify_credential(&cred_path, secret).unwrap();

        let mut c = Config::default();
        c.terminal.notify_chat_id = Some("42".into());
        let config_path = config_path_override(Some(&dir));
        c.save_to(&config_path).unwrap();
        let config_text = std::fs::read_to_string(&config_path).unwrap();

        assert!(
            !config_text.contains(secret),
            "config.toml must never carry the Telegram credential: {config_text}"
        );
        assert!(
            !config_text.to_lowercase().contains("telegram"),
            "config.toml must carry no telegram-shaped field at all: {config_text}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Defect (3): two genuinely concurrent saves in the same process must
    /// not collide on the temp file name. Under the historical bug (the
    /// name derived only from `std::process::id()`, fixed for the life of
    /// the process) two threads racing `save_notify_credential` would
    /// deterministically have one `create_new` fail with `AlreadyExists` —
    /// permanently, since nothing ever cleans that name up. A fresh random
    /// suffix per call makes the two temp paths different, so both calls
    /// succeed regardless of scheduling.
    #[test]
    fn concurrent_saves_never_collide_on_the_temp_name() {
        let dir =
            std::env::temp_dir().join(format!("waggledance-cfg-cred-race-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = notify_credential_path_override(Some(&dir));

        let path_a = path.clone();
        let path_b = path.clone();
        let a = std::thread::spawn(move || save_notify_credential(&path_a, "secret-a"));
        let b = std::thread::spawn(move || save_notify_credential(&path_b, "secret-b"));
        let result_a = a.join().unwrap();
        let result_b = b.join().unwrap();

        assert!(
            result_a.is_ok(),
            "first concurrent save must succeed: {result_a:?}"
        );
        assert!(
            result_b.is_ok(),
            "second concurrent save must succeed: {result_b:?}"
        );
        // Whichever rename lands last wins the final content; either is
        // fine — the point proven here is that neither call itself failed.
        let final_value = load_notify_credential(&path);
        assert!(
            final_value.as_deref() == Some("secret-a")
                || final_value.as_deref() == Some("secret-b"),
            "credential must hold whichever concurrent save landed last, got {final_value:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Defect (3), the other half: a temp file left behind by a crash
    /// before its rename landed (a real scenario under the old fixed-name
    /// scheme — `std::process::id()` is identical across a restart within
    /// the same short-lived process, so a stale temp file from a previous
    /// run could sit at the exact path a later save would also pick) must
    /// never block a later save from succeeding.
    #[test]
    fn a_leftover_temp_file_never_blocks_a_later_save() {
        let dir = std::env::temp_dir().join(format!(
            "waggledance-cfg-cred-leftover-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = notify_credential_path_override(Some(&dir));

        // Simulate the historical bug's leftover: the fixed pid-derived
        // name a pre-fix build would have used, never cleaned up because
        // its rename never ran.
        let stale = dir.join(format!(
            "{NOTIFY_CREDENTIAL_FILE_NAME}.tmp-{}",
            std::process::id()
        ));
        std::fs::write(&stale, b"leftover from a crashed save").unwrap();

        let result = save_notify_credential(&path, "fresh-secret");
        assert!(
            result.is_ok(),
            "a leftover temp file must never block a later save: {result:?}"
        );
        assert_eq!(
            load_notify_credential(&path).as_deref(),
            Some("fresh-secret")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Defect (2): a save that genuinely cannot write must report the
    /// failure, not silently succeed — the function-level contract that
    /// underlies the "surface, don't discard" fix, provable independent of
    /// whatever a given caller does with the returned `Result`.
    #[cfg(unix)]
    #[test]
    fn a_save_that_cannot_write_reports_the_failure() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "waggledance-cfg-cred-denied-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = notify_credential_path_override(Some(&dir));
        // No write permission on the containing directory: `create_new`
        // cannot create the temp file at all.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();

        let result = save_notify_credential(&path, "should-never-land");

        // Restore write permission before cleanup, or `remove_dir_all` fails too.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(
            result.is_err(),
            "a save that cannot write must report failure, not Ok(())"
        );
        assert_eq!(
            load_notify_credential(&path),
            None,
            "a failed save must never leave a partial credential readable"
        );
    }

    /// Defect (4): originally added because the identical mechanism next
    /// door — the agent terminal's own token file, removed with
    /// `terminal_auth` (toa-3, D1) — carried this assertion and the
    /// credential file had none. Catches a regression that drops the
    /// owner-only mode.
    #[cfg(unix)]
    #[test]
    fn credential_file_is_created_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("waggledance-cfg-cred-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = notify_credential_path_override(Some(&dir));
        save_notify_credential(&path, "mode-check-secret").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "telegram credential file must be owner-read/write only"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
