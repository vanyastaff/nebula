//! Configuration presets for common scenarios

use super::{Config, DisplayConfig, Fields, Format};

impl Config {
    /// Create configuration from environment variables
    #[must_use]
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
    #[must_use]
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
    #[must_use]
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
