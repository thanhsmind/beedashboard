//! Config (`~/.mdview/config.toml`). Atomic write, resilient load (corrupt → default).
//! Mirrors PRD §10.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
/// user turns them on from the settings page. The terminal token itself is
/// deliberately **not** a field here (P1, `mdview/src/terminal_auth.rs`):
/// `Config` is serialized whole and unauthenticated by `GET /api/config`, so
/// anything stored inside it is one request away regardless of what the
/// settings HTML masks. `#[derive(Default)]` gives every switch `false`,
/// matching a config that has never seen this section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// The terminal surface itself (D2/D3) — panes and screens are reachable
    /// only once this is on.
    pub enabled: bool,
    /// D7: keep the herdr supervisor process alive. mdview spawns nothing
    /// while this is off.
    pub supervisor_enabled: bool,
    /// D7: Telegram notification on agent status change. mdview makes no
    /// outbound call while this is off.
    pub notify_enabled: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 7700,
            // Bind all interfaces by default so the viewer is reachable from
            // other devices on the LAN (and from a browser when the daemon runs
            // on a remote host). The server has no auth; `serve()` prints a
            // non-loopback exposure warning at startup.
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
/// `~/.mdview/` — the app data directory (created on demand).
pub fn data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mdview")
}

/// `data_dir()`, or `override_dir` when given. Callers that must be testable
/// without touching the developer's real `~/.mdview` (route handlers exercised
/// through a test harness) resolve the data directory through this instead of
/// calling `data_dir()` directly. With `override_dir` unset this returns
/// exactly what `data_dir()` returns.
pub fn resolve_data_dir(override_dir: Option<&Path>) -> PathBuf {
    override_dir.map(Path::to_path_buf).unwrap_or_else(data_dir)
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

/// `config_path()`, or `override_dir/config.toml` when given.
pub fn config_path_override(override_dir: Option<&Path>) -> PathBuf {
    resolve_data_dir(override_dir).join("config.toml")
}

pub fn registry_db_path() -> PathBuf {
    data_dir().join("registry.db")
}

pub fn daemon_lock_path() -> PathBuf {
    data_dir().join("daemon.lock")
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
        let dir = std::env::temp_dir().join(format!("mdview-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, "this is not = valid : toml ][").unwrap();
        let c = Config::load_from(&p);
        assert_eq!(c.server.port, 7700);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn roundtrip_atomic_save_load() {
        let dir = std::env::temp_dir().join(format!("mdview-cfg2-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("mdview-cfg-override-{}", std::process::id()));
        assert_eq!(resolve_data_dir(Some(&dir)), dir);
        assert_eq!(
            config_path_override(Some(&dir)),
            dir.join("config.toml")
        );
    }

    #[test]
    fn resolve_data_dir_falls_back_to_data_dir_when_unset() {
        assert_eq!(resolve_data_dir(None), data_dir());
        assert_eq!(config_path_override(None), config_path());
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

        // Round-trips through TOML with no token anywhere in the section.
        let dir = std::env::temp_dir().join(format!("mdview-cfg-terminal-{}", std::process::id()));
        let p = dir.join("config.toml");
        c.save_to(&p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("[terminal]"));
        assert!(!text.to_lowercase().contains("token"));
        let loaded = Config::load_from(&p);
        assert!(!loaded.terminal.enabled);
        assert!(!loaded.terminal.supervisor_enabled);
        assert!(!loaded.terminal.notify_enabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hostname_defaults_to_none_and_roundtrips_when_set() {
        assert_eq!(ServerConfig::default().hostname, None);

        let dir = std::env::temp_dir().join(format!("mdview-cfg3-{}", std::process::id()));
        let p = dir.join("config.toml");
        let mut c = Config::default();
        c.server.hostname = Some("my-machine.local".into());
        c.save_to(&p).unwrap();
        let loaded = Config::load_from(&p);
        assert_eq!(loaded.server.hostname.as_deref(), Some("my-machine.local"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
