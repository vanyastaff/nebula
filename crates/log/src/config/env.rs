//! Environment precedence resolution.

use super::{Config, Fields, Format};

/// Source used to resolve startup configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedSource {
    /// Explicit runtime config passed by the caller.
    Explicit,
    /// Environment overrides on top of preset config.
    Environment,
    /// Preset defaults.
    Preset,
}

/// Result of startup config resolution.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Effective config after precedence resolution.
    pub config: Config,
    /// Highest-priority source used.
    pub source: ResolvedSource,
}

fn parse_bool(value: &str) -> bool {
    !matches!(value, "0" | "false" | "FALSE" | "False")
}

fn parse_format(value: &str) -> Option<Format> {
    match value.to_lowercase().as_str() {
        "pretty" => Some(Format::Pretty),
        "json" => Some(Format::Json),
        "logfmt" => Some(Format::Logfmt),
        "compact" => Some(Format::Compact),
        _ => None,
    }
}

impl Config {
    /// Apply environment variable overrides to an existing configuration.
    ///
    /// Returns `true` if at least one environment variable was applied.
    pub fn apply_env_overrides(&mut self) -> bool {
        let mut applied = false;

        if let Ok(level) = std::env::var("NEBULA_LOG") {
            self.level = level;
            applied = true;
        } else if let Ok(level) = std::env::var("RUST_LOG") {
            self.level = level;
            applied = true;
        }

        if let Ok(format) = std::env::var("NEBULA_LOG_FORMAT")
            && let Some(parsed) = parse_format(&format)
        {
            self.format = parsed;
            applied = true;
        }

        if let Ok(v) = std::env::var("NEBULA_LOG_TIME") {
            self.display.time = parse_bool(&v);
            applied = true;
        }
        if let Ok(v) = std::env::var("NEBULA_LOG_SOURCE") {
            self.display.source = parse_bool(&v);
            applied = true;
        }
        if let Ok(v) = std::env::var("NEBULA_LOG_COLORS") {
            self.display.colors = parse_bool(&v);
            applied = true;
        }

        let env_fields = Fields::from_env();
        if !env_fields.is_empty() {
            self.fields = env_fields;
            applied = true;
        }

        applied
    }

    /// Resolve configuration with precedence: explicit > env > preset.
    #[must_use]
    pub fn resolve_startup(explicit: Option<Self>) -> ResolvedConfig {
        if let Some(config) = explicit {
            return ResolvedConfig {
                config,
                source: ResolvedSource::Explicit,
            };
        }

        let mut config = if cfg!(debug_assertions) {
            Self::development()
        } else {
            Self::production()
        };

        if config.apply_env_overrides() {
            ResolvedConfig {
                config,
                source: ResolvedSource::Environment,
            }
        } else {
            ResolvedConfig {
                config,
                source: ResolvedSource::Preset,
            }
        }
    }
}
