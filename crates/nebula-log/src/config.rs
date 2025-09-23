//! Configuration types and builders

use serde::{Deserialize, Serialize};

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

/// Writer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WriterConfig {
    /// Write to stderr
    Stderr,
    /// Write to stdout
    Stdout,
    /// Write to file
    #[cfg(feature = "file")]
    File {
        path: std::path::PathBuf,
        #[serde(default)]
        rolling: Option<Rolling>,
        #[serde(default = "default_non_blocking")]
        non_blocking: bool,
    },
    /// Write to multiple destinations
    Multi(Vec<WriterConfig>),
}

/// File rolling strategy
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    /// Show timestamps
    pub time: bool,
    /// Custom time format (strftime)
    pub time_format: Option<String>,
    /// Show source location (file:line)
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

/// Global fields configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Fields {
    /// Service name
    pub service: Option<String>,
    /// Environment (dev/staging/prod)
    pub env: Option<String>,
    /// Version
    pub version: Option<String>,
    /// Instance ID
    pub instance: Option<String>,
    /// Region
    pub region: Option<String>,
    /// Custom fields
    #[serde(flatten)]
    pub custom: std::collections::BTreeMap<String, serde_json::Value>,
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

// ============================================================================
// Implementations
// ============================================================================

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

impl Config {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Parse NEBULA_LOG or RUST_LOG
        if let Ok(level) = std::env::var("NEBULA_LOG") {
            config.level = level;
        } else if let Ok(level) = std::env::var("RUST_LOG") {
            config.level = level;
        }

        // Parse format
        if let Ok(format) = std::env::var("NEBULA_LOG_FORMAT") {
            config.format = match format.to_lowercase().as_str() {
                "pretty" => Format::Pretty,
                "json" => Format::Json,
                "logfmt" => Format::Logfmt,
                _ => Format::Compact,
            };
        }

        // Parse display options
        config.display.parse_env();

        // Parse fields from env
        config.fields = Fields::from_env();

        config
    }

    /// Development configuration (pretty, debug level)
    pub fn development() -> Self {
        Self {
            level: "debug".to_string(),
            format: Format::Pretty,
            display: DisplayConfig {
                colors: true,
                source: true,
                ..DisplayConfig::default()
            },
            ..Self::default()
        }
    }

    /// Production configuration (JSON, info level)
    pub fn production() -> Self {
        Self {
            level: "info".to_string(),
            format: Format::Json,
            display: DisplayConfig {
                colors: false,
                source: false,
                flatten: true,
                ..DisplayConfig::default()
            },
            ..Self::default()
        }
    }

    /// Test configuration (captures output)
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            level: "trace".to_string(),
            format: Format::Compact,
            display: DisplayConfig {
                colors: false,
                time: false,
                ..DisplayConfig::default()
            },
            ..Self::default()
        }
    }
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
    fn parse_env(&mut self) {
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

impl Fields {
    /// Create fields from environment variables
    pub fn from_env() -> Self {
        Self {
            service: std::env::var("NEBULA_SERVICE").ok(),
            env: std::env::var("NEBULA_ENV").ok(),
            version: std::env::var("NEBULA_VERSION")
                .ok()
                .or_else(|| option_env!("CARGO_PKG_VERSION").map(String::from)),
            instance: std::env::var("NEBULA_INSTANCE").ok(),
            region: std::env::var("NEBULA_REGION").ok(),
            custom: Default::default(),
        }
    }

    /// Check if fields are empty
    pub fn is_empty(&self) -> bool {
        self.service.is_none()
            && self.env.is_none()
            && self.version.is_none()
            && self.instance.is_none()
            && self.region.is_none()
            && self.custom.is_empty()
    }
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self::Stderr
    }
}

#[allow(dead_code)]
fn default_non_blocking() -> bool {
    true
}

