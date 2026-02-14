//! Writer and display configuration

use serde::{Deserialize, Serialize};

/// Writer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
#[derive(Default)]
#[non_exhaustive]
pub enum WriterConfig {
    /// Write to stderr
    #[default]
    Stderr,
    /// Write to stdout
    Stdout,
    /// Write to file
    #[cfg(feature = "file")]
    File {
        /// Path to the log file
        path: std::path::PathBuf,
        /// Rolling policy for log rotation
        #[serde(default)]
        rolling: Option<Rolling>,
        /// Whether to use non-blocking writer
        #[serde(default = "default_non_blocking")]
        non_blocking: bool,
    },
    /// Write to multiple destinations
    Multi(Vec<WriterConfig>),
}

/// File rolling strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Rolling {
    /// Never roll files
    Never,
    /// Roll hourly
    Hourly,
    /// Roll daily
    Daily,
    /// Roll by size in MB (not yet implemented)
    Size(u64),
}

/// Display configuration
///
/// This struct uses multiple boolean flags for configuration as it represents
/// independent toggleable features. This is more ergonomic than enums or bitflags
/// for a configuration struct that maps directly to CLI flags or config files.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Show timestamps
    pub time: bool,
    /// Custom time format (strftime)
    pub time_format: Option<String>,
    /// Show source location (`file:line`)
    pub source: bool,
    /// Show target module
    pub target: bool,
    /// Show thread IDs
    pub thread_ids: bool,
    /// Show thread names
    pub thread_names: bool,
    /// Use ANSI colors
    pub colors: bool,
    /// Show span list in JSON
    pub span_list: bool,
    /// Flatten JSON events
    pub flatten: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            time: true,
            time_format: None,
            source: cfg!(debug_assertions),
            target: true,
            thread_ids: false,
            thread_names: false,
            colors: cfg!(feature = "ansi") && std::io::IsTerminal::is_terminal(&std::io::stderr()),
            span_list: true,
            flatten: true,
        }
    }
}

impl DisplayConfig {
    /// Parse display configuration from environment variables
    pub(super) fn parse_env(&mut self) {
        if let Ok(v) = std::env::var("NEBULA_LOG_TIME") {
            self.time = v != "0" && v != "false";
        }
        if let Ok(v) = std::env::var("NEBULA_LOG_SOURCE") {
            self.source = v != "0" && v != "false";
        }
        if let Ok(v) = std::env::var("NEBULA_LOG_COLORS") {
            self.colors = v != "0" && v != "false";
        }
    }
}

#[allow(dead_code)]
fn default_non_blocking() -> bool {
    true
}
