//! CLI configuration loaded from `nebula.toml` files and environment variables.
//!
//! Resolution order (later overrides earlier):
//! 1. Built-in defaults
//! 2. `~/.nebula/config.toml` (user-global)
//! 3. `./nebula.toml` (project-local)
//! 4. Environment variables (`NEBULA_*`)
//! 5. CLI flags (highest priority)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

impl CliConfig {
    /// Load configuration from standard locations.
    ///
    /// Merges: defaults → ~/.nebula/config.toml → ./nebula.toml → env vars.
    pub async fn load() -> Self {
        let mut builder = nebula_config::ConfigBuilder::new().with_defaults(default_config_json());

        // User-global config.
        if let Some(global_path) = global_config_path()
            && global_path.exists()
        {
            builder = builder.with_source(nebula_config::ConfigSource::File(global_path));
        }

        // Project-local config.
        let local_path = PathBuf::from("nebula.toml");
        if local_path.exists() {
            builder = builder.with_source(nebula_config::ConfigSource::File(local_path));
        }

        match builder.build().await {
            Ok(config) => config.get_all::<CliConfig>().await.unwrap_or_default(),
            Err(_) => Self::default(),
        }
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
# api_key = "${NEBULA_API_KEY}"
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

fn default_config_json() -> serde_json::Value {
    serde_json::to_value(CliConfig::default()).expect("default config serialization")
}
