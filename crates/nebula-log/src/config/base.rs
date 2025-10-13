//! Core configuration types

use serde::{Deserialize, Serialize};

use super::{DisplayConfig, Fields, WriterConfig};

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
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
