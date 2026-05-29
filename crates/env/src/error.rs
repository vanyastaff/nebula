//! Error type for environment-variable resolution.

/// Failure reading or parsing an environment variable.
///
/// Consumers map this into their own typed error at the boundary
/// (e.g. `ApiConfigError`, `ProviderError`) rather than surfacing it
/// directly — `nebula-env` is shared infra, not a public API contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EnvError {
    /// A required variable is not set.
    #[error("environment variable `{var}` is not set")]
    Missing {
        /// Variable name.
        var: String,
    },

    /// The variable is set but its value is not valid Unicode.
    #[error("environment variable `{var}` is not valid Unicode")]
    NotUnicode {
        /// Variable name.
        var: String,
    },

    /// The variable is set but failed to parse into the requested type.
    #[error("environment variable `{var}` could not be parsed: {message}")]
    Parse {
        /// Variable name.
        var: String,
        /// `Display` of the underlying parse error.
        message: String,
    },

    /// The variable is set to a value outside the accepted set.
    #[error("environment variable `{var}` has invalid value `{value}` (expected {expected})")]
    Invalid {
        /// Variable name.
        var: String,
        /// The rejected value.
        value: String,
        /// Human-readable description of the accepted values.
        expected: &'static str,
    },
}
