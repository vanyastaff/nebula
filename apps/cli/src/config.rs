//! CLI configuration loaded from `nebula.toml` files and environment variables.
//!
//! Resolution order (later overrides earlier):
//! 1. Built-in defaults
//! 2. `~/.config/nebula/config.toml` (user-global, via `dirs::config_dir`)
//! 3. `./nebula.toml` (project-local)
//! 4. Environment variables (see below)
//! 5. CLI flags (highest priority; applied by the caller after `load()` returns)
//!
//! ## Environment variable convention
//!
//! Variables are prefixed with `NEBULA_`. Use **double underscore (`__`)** as
//! the path separator between the section and the field name. Single underscores
//! are preserved as part of field names.
//!
//! | Variable                        | Config path              |
//! |---------------------------------|--------------------------|
//! | `NEBULA_RUN__CONCURRENCY`       | `run.concurrency`        |
//! | `NEBULA_RUN__TIMEOUT_SECS`      | `run.timeout_secs`       |
//! | `NEBULA_RUN__FORMAT`            | `run.format`             |
//! | `NEBULA_LOG__LEVEL`             | `log.level`              |
//! | `NEBULA_REMOTE__URL`            | `remote.url`             |
//! | `NEBULA_REMOTE__API_KEY`        | `remote.api_key`         |
//!
//! The general pattern is `NEBULA_{SECTION}__{FIELD}` where `{FIELD}` is the
//! Rust field name verbatim (underscores intact). Top-level fields with no
//! section use just `NEBULA__{FIELD}`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

/// CLI configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    /// Execution defaults.
    pub run: RunConfig,
    /// Remote server settings (for future API client mode).
    pub remote: Option<RemoteConfig>,
    /// Logging configuration.
    pub log: LogConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConfig {
    /// Default max concurrent nodes.
    pub concurrency: usize,
    /// Default execution timeout in seconds. `None` = unlimited.
    pub timeout_secs: Option<u64>,
    /// Default output format.
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Server URL.
    pub url: String,
    /// API key for authentication.
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// Default log level.
    pub level: String,
}

// ── Defaults ─────────────────────────────────────────────────────────────

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            concurrency: 10,
            timeout_secs: None,
            format: "json".to_owned(),
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "error".to_owned(),
        }
    }
}

// ── Loading ──────────────────────────────────────────────────────────────

/// Read a TOML config file, failing loudly on parse errors.
///
/// Returns `Ok(None)` if the file does not exist (both config files are
/// optional). Returns `Err` if the file exists but cannot be read or is
/// syntactically invalid TOML — we never silently fall back to defaults when
/// the user has a broken config file.
fn read_toml_file(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    // Validate TOML syntax before handing to figment.
    toml::from_str::<toml::Value>(&contents)
        .with_context(|| format!("invalid TOML in config file: {}", path.display()))?;
    Ok(Some(contents))
}

impl CliConfig {
    /// Load configuration from standard locations.
    ///
    /// Merges: defaults → global TOML → local `nebula.toml` → `NEBULA_**` env vars.
    ///
    /// Returns an error if a config file exists but contains invalid TOML.
    /// Missing config files are silently skipped (they are optional).
    pub async fn load() -> anyhow::Result<Self> {
        let mut fig = Figment::from(Serialized::defaults(CliConfig::default()));

        if let Some(global_path) = global_config_path()
            && let Some(contents) = read_toml_file(&global_path)?
        {
            fig = fig.merge(Toml::string(&contents));
        }

        let local_path = PathBuf::from("nebula.toml");
        if let Some(contents) = read_toml_file(&local_path)? {
            fig = fig.merge(Toml::string(&contents));
        }

        // `NEBULA_RUN__TIMEOUT_SECS` → `run.timeout_secs`, etc.
        // Double underscore is the path separator; single underscores within a
        // segment are preserved as part of the field name.
        fig = fig.merge(Env::prefixed("NEBULA_").split("__"));

        fig.extract().context("failed to extract CLI configuration")
    }

    /// Generate the default config file content as TOML.
    pub fn default_toml() -> String {
        r#"# Nebula CLI configuration
# Global: ~/.config/nebula/config.toml (Linux), ~/Library/Application Support/nebula/config.toml (macOS)
# Project: ./nebula.toml

[run]
# Default maximum concurrent nodes
concurrency = 10
# Default output format: "json" or "text"
format = "json"
# Default timeout in seconds (commented = unlimited)
# timeout_secs = 300

[log]
# Default log level: "error", "warn", "info", "debug", "trace"
level = "error"

# [remote]
# url = "https://nebula.example.com"
# api_key = "${NEBULA_REMOTE__API_KEY}"
"#
        .to_owned()
    }
}

/// Path to the global config file.
///
/// - Linux:   `~/.config/nebula/config.toml`
/// - macOS:   `~/Library/Application Support/nebula/config.toml`
/// - Windows: `C:\Users\<user>\AppData\Roaming\nebula\config.toml`
pub fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("nebula").join("config.toml"))
}

/// Path to the global config directory.
pub fn global_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("nebula"))
}

/// Check if a config file exists at the standard locations.
pub fn find_config_file() -> Option<PathBuf> {
    let local = PathBuf::from("nebula.toml");
    if local.exists() {
        return Some(local);
    }
    let global = global_config_path()?;
    if global.exists() {
        return Some(global);
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: run the async load() synchronously inside tests.
    fn load_sync() -> anyhow::Result<CliConfig> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(CliConfig::load())
    }

    #[test]
    fn defaults_when_no_config_files() {
        // Make sure no nebula.toml is present in the working directory by
        // running from a temp dir with no config files.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = std::env::set_current_dir(tmp.path());
        let cfg = load_sync().expect("load should succeed with defaults");
        assert_eq!(cfg.run.concurrency, 10);
        assert_eq!(cfg.run.format, "json");
        assert!(cfg.run.timeout_secs.is_none());
        assert_eq!(cfg.log.level, "error");
        assert!(cfg.remote.is_none());
    }

    #[test]
    fn env_var_double_underscore_timeout_secs() {
        // NEBULA_RUN__TIMEOUT_SECS must land in run.timeout_secs, not
        // a non-existent run.timeout.secs path.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = std::env::set_current_dir(tmp.path());
        // SAFETY: single-threaded test runtime; no concurrent env access.
        unsafe { std::env::set_var("NEBULA_RUN__TIMEOUT_SECS", "42") };
        let cfg = load_sync().expect("load should succeed");
        // SAFETY: single-threaded test runtime; no concurrent env access.
        unsafe { std::env::remove_var("NEBULA_RUN__TIMEOUT_SECS") };
        assert_eq!(cfg.run.timeout_secs, Some(42));
    }

    #[test]
    fn env_var_double_underscore_api_key() {
        // NEBULA_REMOTE__API_KEY must land in remote.api_key, not remote.api.key.
        let tmp = tempfile::tempdir().unwrap();
        let _guard = std::env::set_current_dir(tmp.path());
        // SAFETY: single-threaded test runtime; no concurrent env access.
        unsafe {
            std::env::set_var("NEBULA_REMOTE__URL", "https://example.com");
            std::env::set_var("NEBULA_REMOTE__API_KEY", "test-key-abc");
        }
        let cfg = load_sync().expect("load should succeed");
        // SAFETY: single-threaded test runtime; no concurrent env access.
        unsafe {
            std::env::remove_var("NEBULA_REMOTE__URL");
            std::env::remove_var("NEBULA_REMOTE__API_KEY");
        }
        let remote = cfg.remote.expect("remote should be populated");
        assert_eq!(remote.api_key.as_deref(), Some("test-key-abc"));
    }

    #[test]
    fn invalid_toml_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nebula.toml");
        std::fs::write(&path, "this is [not valid toml = {{{{").unwrap();
        let _guard = std::env::set_current_dir(tmp.path());
        let result = load_sync();
        assert!(result.is_err(), "expected error for invalid TOML");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("nebula.toml"),
            "error message should name the file; got: {msg}"
        );
    }

    #[test]
    fn valid_toml_overrides_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nebula.toml");
        std::fs::write(
            &path,
            "[run]\nconcurrency = 4\nformat = \"text\"\n[log]\nlevel = \"debug\"\n",
        )
        .unwrap();
        let _guard = std::env::set_current_dir(tmp.path());
        let cfg = load_sync().expect("load should succeed with valid TOML");
        assert_eq!(cfg.run.concurrency, 4);
        assert_eq!(cfg.run.format, "text");
        assert_eq!(cfg.log.level, "debug");
    }
}
