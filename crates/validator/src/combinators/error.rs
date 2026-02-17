//! Unified error handling for combinators
//!
//! This module provides a unified error type for all combinators,
//! making error handling consistent and composable across the library.
//!
//! # Design Goals
//!
//! - **Unified**: Single error type for all combinators
//! - **Composable**: Easy to combine and nest errors
//! - **Debuggable**: Rich context for error diagnosis
//! - **Interoperable**: Converts to/from `ValidationError`
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::combinators::error::CombinatorError;
//!
//! // Create an OR error with both alternatives failing
//! let error = CombinatorError::or_all_failed(left_err, right_err);
//!
//! // Create a field validation error
//! let error = CombinatorError::field_failed("email", validation_err);
//! ```

use crate::foundation::ValidationError;

// ============================================================================
// COMBINATOR ERROR TYPE
// ============================================================================

/// Unified error type for all combinators.
///
/// This enum captures different failure modes across all combinators,
/// providing a consistent error handling experience.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CombinatorError<E: std::error::Error + Clone + 'static = ValidationError> {
    /// OR combinator: all alternatives failed.
    ///
    /// Contains errors from left and right validators.
    #[error("All validators failed. Left: {left}; Right: {right}")]
    OrAllFailed {
        #[source]
        left: Box<E>,
        right: Box<E>,
    },

    /// AND combinator: one or both validators failed.
    ///
    /// Contains error from the validator that failed.
    #[error("AND combinator failed: {0}")]
    AndFailed(#[source] E),

    /// NOT combinator: validator unexpectedly passed.
    ///
    /// The NOT combinator inverts validation logic - it fails when
    /// the inner validator succeeds.
    #[error("Validation must NOT pass, but it did")]
    NotValidatorPassed,

    /// Field validation failed.
    ///
    /// Contains field name and the validation error for that field.
    #[error("{}", match field_name {
        Some(name) => format!("Validation failed for field '{}': {}", name, error),
        None => format!("Validation failed for field: {}", error),
    })]
    FieldFailed {
        field_name: Option<String>,
        #[source]
        error: Box<E>,
    },

    /// Required value was None.
    ///
    /// Used by Optional/Required combinators when a value is required
    /// but None was provided.
    #[error("Value is required but was None")]
    RequiredValueMissing,

    /// Inner validator failed with an error.
    ///
    /// Generic wrapper for errors from inner validators.
    #[error("Validation failed: {0}")]
    ValidationFailed(#[source] E),

    /// Multiple validators failed.
    ///
    /// Used when validating multiple items or fields where several
    /// can fail independently.
    #[error("Multiple validations failed ({} errors)", .0.len())]
    MultipleFailed(Vec<E>),

    /// Custom error with a message.
    ///
    /// For cases not covered by other variants.
    #[error("[{code}] {message}")]
    Custom { code: String, message: String },
}

// ============================================================================
// CONSTRUCTOR HELPERS
// ============================================================================

impl<E: std::error::Error + Clone + 'static> CombinatorError<E> {
    /// Creates an OR error when all alternatives fail.
    pub fn or_all_failed(left: E, right: E) -> Self {
        Self::OrAllFailed {
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Creates an AND error.
    pub fn and_failed(error: E) -> Self {
        Self::AndFailed(error)
    }

    /// Creates a NOT error when validator unexpectedly passes.
    #[must_use]
    pub fn not_passed() -> Self {
        Self::NotValidatorPassed
    }

    /// Creates a field validation error.
    pub fn field_failed(field_name: impl Into<String>, error: E) -> Self {
        Self::FieldFailed {
            field_name: Some(field_name.into()),
            error: Box::new(error),
        }
    }

    /// Creates a field validation error without a field name.
    pub fn field_failed_unnamed(error: E) -> Self {
        Self::FieldFailed {
            field_name: None,
            error: Box::new(error),
        }
    }

    /// Creates a required value missing error.
    #[must_use]
    pub fn required_missing() -> Self {
        Self::RequiredValueMissing
    }

    /// Creates a validation failed error.
    pub fn validation_failed(error: E) -> Self {
        Self::ValidationFailed(error)
    }

    /// Creates a multiple failures error.
    #[must_use]
    pub fn multiple_failed(errors: Vec<E>) -> Self {
        Self::MultipleFailed(errors)
    }

    /// Creates a custom error.
    pub fn custom(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Custom {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Returns the field name if this is a field error.
    pub fn field_name(&self) -> Option<&str> {
        match self {
            Self::FieldFailed { field_name, .. } => field_name.as_deref(),
            _ => None,
        }
    }

    /// Checks if this error is a field error.
    pub fn is_field_error(&self) -> bool {
        matches!(self, Self::FieldFailed { .. })
    }

    /// Checks if this error contains multiple failures.
    pub fn is_multiple(&self) -> bool {
        matches!(self, Self::MultipleFailed(_))
    }
}

// ============================================================================
// CONVERSION FROM VALIDATIONERROR
// ============================================================================

impl From<ValidationError> for CombinatorError<ValidationError> {
    fn from(error: ValidationError) -> Self {
        Self::ValidationFailed(error)
    }
}

// ============================================================================
// CONVERSION TO VALIDATIONERROR
// ============================================================================

impl<E> From<CombinatorError<E>> for ValidationError
where
    E: std::error::Error + Clone + 'static,
{
    fn from(error: CombinatorError<E>) -> Self {
        match error {
            CombinatorError::OrAllFailed { left, right } => ValidationError::new(
                "or_all_failed",
                format!("All validators failed. Left: {left}; Right: {right}"),
            ),
            CombinatorError::AndFailed(e) => {
                ValidationError::new("and_failed", format!("AND combinator failed: {e}"))
            }
            CombinatorError::NotValidatorPassed => ValidationError::new(
                "not_validator_passed",
                "Validation must NOT pass, but it did",
            ),
            CombinatorError::FieldFailed { field_name, error } => {
                let mut ve = ValidationError::new(
                    "field_validation_failed",
                    format!("Field validation failed: {error}"),
                );
                if let Some(name) = field_name {
                    ve = ve.with_field(name);
                }
                ve
            }
            CombinatorError::RequiredValueMissing => {
                ValidationError::new("required", "Value is required but was None")
            }
            CombinatorError::ValidationFailed(e) => {
                ValidationError::new("validation_failed", format!("{e}"))
            }
            CombinatorError::MultipleFailed(errors) => {
                let mut ve = ValidationError::new(
                    "multiple_failures",
                    format!("Multiple validations failed ({} errors)", errors.len()),
                );
                // Add nested errors
                let nested: Vec<ValidationError> = errors
                    .into_iter()
                    .map(|e| ValidationError::new("nested_error", format!("{e}")))
                    .collect();
                ve = ve.with_nested(nested);
                ve
            }
            CombinatorError::Custom { code, message } => ValidationError::new(code, message),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_or_all_failed() {
        let err1 = ValidationError::new("err1", "First error");
        let err2 = ValidationError::new("err2", "Second error");
        let error = CombinatorError::or_all_failed(err1, err2);

        assert!(matches!(error, CombinatorError::OrAllFailed { .. }));
        let display = format!("{}", error);
        assert!(display.contains("All validators failed"));
    }

    #[test]
    fn test_field_failed() {
        let err = ValidationError::new("invalid", "Invalid value");
        let error = CombinatorError::field_failed("email", err);

        assert!(error.is_field_error());
        assert_eq!(error.field_name(), Some("email"));
        let display = format!("{}", error);
        assert!(display.contains("field 'email'"));
    }

    #[test]
    fn test_required_missing() {
        let error: CombinatorError<ValidationError> = CombinatorError::required_missing();
        let display = format!("{}", error);
        assert!(display.contains("required"));
    }

    #[test]
    fn test_not_passed() {
        let error: CombinatorError<ValidationError> = CombinatorError::not_passed();
        let display = format!("{}", error);
        assert!(display.contains("must NOT pass"));
    }

    #[test]
    fn test_multiple_failed() {
        let errors = vec![
            ValidationError::new("err1", "Error 1"),
            ValidationError::new("err2", "Error 2"),
        ];
        let error = CombinatorError::multiple_failed(errors);

        assert!(error.is_multiple());
        let display = format!("{}", error);
        assert!(display.contains("2 errors"));
    }

    #[test]
    fn test_custom_error() {
        let error: CombinatorError<ValidationError> =
            CombinatorError::custom("custom_code", "Custom message");
        let display = format!("{}", error);
        assert!(display.contains("custom_code"));
        assert!(display.contains("Custom message"));
    }

    #[test]
    fn test_conversion_to_validation_error() {
        let error: CombinatorError<ValidationError> = CombinatorError::required_missing();
        let ve: ValidationError = error.into();
        assert_eq!(ve.code, "required");
    }

    #[test]
    fn test_conversion_from_validation_error() {
        let ve = ValidationError::new("test", "Test error");
        let error: CombinatorError<ValidationError> = ve.into();
        assert!(matches!(error, CombinatorError::ValidationFailed(_)));
    }
}
