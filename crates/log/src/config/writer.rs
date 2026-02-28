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
    Multi {
        /// Behavior when one destination fails.
        #[serde(default)]
        policy: DestinationFailurePolicy,
        /// Destination writers.
        writers: Vec<WriterConfig>,
    },
}

/// Failure policy for multi-destination writer behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DestinationFailurePolicy {
    /// Stop and return the first destination error.
    FailFast,
    /// Continue writing to remaining destinations and report best-effort behavior.
    #[default]
    BestEffort,
    /// Try the first destination and fallback to remaining ones on failure.
    PrimaryWithFallback,
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
    /// Roll by size in MB
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

#[allow(dead_code)]
fn default_non_blocking() -> bool {
    true
}
