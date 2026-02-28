//! Configuration presets for common scenarios

use super::{Config, DisplayConfig, Format};

impl Config {
    /// Create configuration from environment variables
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::default();
        let _ = config.apply_env_overrides();
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
