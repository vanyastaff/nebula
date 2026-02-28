//! Core configuration types

use serde::{Deserialize, Serialize};

use crate::core::{LogError, LogResult};

use super::{DisplayConfig, Fields, WriterConfig};

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Configuration schema version.
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,

    /// Log level filter (e.g., "info", "debug,hyper=warn")
    pub level: String,

    /// Output format
    pub format: Format,

    /// Output writer configuration
    pub writer: WriterConfig,

    /// Display configuration
    pub display: DisplayConfig,

    /// Global fields to include in all events
    pub fields: Fields,

    /// Enable runtime reload capability
    pub reloadable: bool,

    /// Telemetry configuration
    #[cfg(feature = "telemetry")]
    pub telemetry: Option<TelemetryConfig>,
}

/// Output format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Format {
    /// Human-readable with colors and indentation
    Pretty,
    /// Compact single-line output
    Compact,
    /// Structured JSON output
    Json,
    /// Logfmt format (key=value pairs)
    Logfmt,
}

/// Log level
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Level {
    /// Trace level
    Trace,
    /// Debug level
    Debug,
    /// Info level
    Info,
    /// Warn level
    Warn,
    /// Error level
    Error,
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Level::Trace => write!(f, "trace"),
            Level::Debug => write!(f, "debug"),
            Level::Info => write!(f, "info"),
            Level::Warn => write!(f, "warn"),
            Level::Error => write!(f, "error"),
        }
    }
}

#[cfg(feature = "telemetry")]
/// Telemetry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// OpenTelemetry endpoint
    pub otlp_endpoint: Option<String>,
    /// Service name for traces
    pub service_name: String,
    /// Sampling rate (0.0-1.0)
    pub sampling_rate: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            level: "info".to_string(),
            format: Format::Compact,
            writer: WriterConfig::Stderr,
            display: DisplayConfig::default(),
            fields: Fields::default(),
            reloadable: false,
            #[cfg(feature = "telemetry")]
            telemetry: None,
        }
    }
}

impl Config {
    /// Validate config schema compatibility.
    ///
    /// # Errors
    ///
    /// Returns `LogError::Config` when schema version is unsupported.
    pub fn ensure_compatible(&self) -> LogResult<()> {
        if self.schema_version != default_schema_version() {
            return Err(LogError::Config(format!(
                "Unsupported config schema version {} (expected {})",
                self.schema_version,
                default_schema_version()
            )));
        }
        Ok(())
    }
}

const fn default_schema_version() -> u16 {
    1
}
