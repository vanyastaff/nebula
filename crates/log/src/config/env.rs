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

/// Tri-state colour selection parsed from `NEBULA_LOG_COLORS`.
///
/// The documented contract is `auto | always | never`; bool-style aliases
/// (`true/1/yes/on`, `false/0/no/off`) are also accepted for back-compat.
/// `auto` and any unrecognized value resolve to [`ColorMode::Auto`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "always" | "true" | "1" | "yes" | "on" => Self::Always,
            "never" | "false" | "0" | "no" | "off" => Self::Never,
            // "auto" and anything unrecognized fall back to TTY auto-detect.
            _ => Self::Auto,
        }
    }

    /// Resolve to a concrete ANSI on/off. `Auto` enables colours only when
    /// stderr is a terminal; every arm still requires the `ansi` feature to
    /// emit codes, matching `DisplayConfig`'s default derivation.
    fn resolve(self) -> bool {
        match self {
            Self::Always => cfg!(feature = "ansi"),
            Self::Never => false,
            Self::Auto => {
                cfg!(feature = "ansi") && std::io::IsTerminal::is_terminal(&std::io::stderr())
            },
        }
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
            self.display.colors = ColorMode::parse(&v).resolve();
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

#[cfg(test)]
mod tests {
    use super::ColorMode;

    #[test]
    fn color_mode_parses_documented_contract() {
        assert_eq!(ColorMode::parse("auto"), ColorMode::Auto);
        assert_eq!(ColorMode::parse("always"), ColorMode::Always);
        assert_eq!(ColorMode::parse("never"), ColorMode::Never);
    }

    #[test]
    fn color_mode_accepts_bool_aliases_case_insensitively() {
        for on in ["true", "1", "YES", "On"] {
            assert_eq!(ColorMode::parse(on), ColorMode::Always, "{on}");
        }
        for off in ["false", "0", "no", "OFF", "FALSE"] {
            assert_eq!(ColorMode::parse(off), ColorMode::Never, "{off}");
        }
    }

    #[test]
    fn color_mode_unrecognized_falls_back_to_auto() {
        assert_eq!(ColorMode::parse("maybe"), ColorMode::Auto);
        assert_eq!(ColorMode::parse(""), ColorMode::Auto);
    }

    #[test]
    fn never_resolves_to_disabled() {
        // Regression: the old `parse_bool` returned `true` for "never"
        // (it is not in the false-set), silently enabling colours.
        assert!(!ColorMode::Never.resolve());
    }
}
