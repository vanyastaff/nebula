//! Crate-level operational error type.
//!
//! [`ValidatorError`] represents errors that occur during validator
//! construction or configuration — as opposed to [`ValidationError`](crate::foundation::ValidationError),
//! which represents validation *failures* on user input.
//!
//! # When to Use
//!
//! - `ValidatorError` — invalid regex in `MatchesRegex::new()`, bad range bounds, etc.
//! - `ValidationError` — user input that does not satisfy a rule.

use std::borrow::Cow;

/// Crate-level operational error.
///
/// Covers errors that occur during validator setup or proof-token
/// construction, not during validation of user data.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::error::ValidatorError;
///
/// let err = ValidatorError::invalid_config("min must be <= max");
/// assert_eq!(err.to_string(), "invalid configuration: min must be <= max");
/// ```
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum ValidatorError {
    /// Invalid validator configuration (e.g., `min > max`).
    #[classify(category = "validation", code = "VALIDATOR:INVALID_CONFIG", retryable = false)]
    #[error("invalid configuration: {message}")]
    InvalidConfig {
        /// Human-readable description of the configuration problem.
        message: Cow<'static, str>,
    },

    /// A validation failure wrapped as an operational error.
    ///
    /// Used when a proof token cannot be issued because validation failed.
    #[classify(category = "validation", code = "VALIDATOR:VALIDATION_FAILED", retryable = false)]
    #[error("validation failed: {0}")]
    ValidationFailed(#[from] crate::foundation::ValidationError),
}

/// Result type alias for [`ValidatorError`].
pub type ValidatorResult<T> = Result<T, ValidatorError>;

impl ValidatorError {
    /// Creates an [`InvalidConfig`](Self::InvalidConfig) error.
    pub fn invalid_config(message: impl Into<Cow<'static, str>>) -> Self {
        Self::InvalidConfig {
            message: message.into(),
        }
    }
}
