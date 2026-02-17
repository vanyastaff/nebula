//! MESSAGE combinator - custom error messages

use std::borrow::Cow;

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// WITH MESSAGE COMBINATOR
// ============================================================================

/// Replaces the error message of a validator.
///
/// Useful for providing user-friendly or localized error messages.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::WithMessage;
/// use nebula_validator::foundation::Validate;
///
/// let validator = WithMessage::new(
///     MinLength { min: 8 },
///     "Password must be at least 8 characters"
/// );
///
/// let result = validator.validate("short");
/// assert_eq!(result.unwrap_err().message, "Password must be at least 8 characters");
/// ```
#[derive(Debug, Clone)]
pub struct WithMessage<V> {
    inner: V,
    message: String,
    code: Option<String>,
}

impl<V> WithMessage<V> {
    /// Creates a new WithMessage combinator with a custom message.
    pub fn new(inner: V, message: impl Into<String>) -> Self {
        Self {
            inner,
            message: message.into(),
            code: None,
        }
    }

    /// Creates a combinator that only overrides the error code, keeping the original message.
    pub fn code_only(inner: V, code: impl Into<String>) -> Self {
        Self {
            inner,
            message: String::new(),
            code: Some(code.into()),
        }
    }

    /// Also replaces the error code.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Returns a reference to the inner validator.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Returns the custom message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the custom code, if set.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Extracts the inner validator.
    pub fn into_inner(self) -> V {
        self.inner
    }
}

impl<V> Validate for WithMessage<V>
where
    V: Validate,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        self.inner.validate(input).map_err(|original| {
            let code = self
                .code
                .clone()
                .map_or_else(|| original.code.clone(), Cow::Owned);

            let message = if self.message.is_empty() {
                original.message.clone()
            } else {
                Cow::Owned(self.message.clone())
            };

            ValidationError::new(code, message).with_nested_error(original)
        })
    }
}

/// Creates a WithMessage combinator.
pub fn with_message<V>(validator: V, message: impl Into<String>) -> WithMessage<V> {
    WithMessage::new(validator, message)
}

// ============================================================================
// WITH CODE (type alias for backwards compatibility)
// ============================================================================

/// Type alias for backwards compatibility.
///
/// `WithCode<V>` is now [`WithMessage<V>`] configured to only override the error code.
///
/// # Warning
///
/// **Do not use `WithCode::new()`** — it is inherited from `WithMessage` and sets the
/// *message*, not the code. Use [`with_code()`] or [`WithMessage::code_only()`] instead.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::with_code;
///
/// // Correct: use the free function
/// let validator = with_code(MinLength { min: 8 }, "ERR_PASSWORD_TOO_SHORT");
///
/// // Also correct: use code_only constructor
/// let validator = WithMessage::code_only(MinLength { min: 8 }, "ERR_PASSWORD_TOO_SHORT");
/// ```
pub type WithCode<V> = WithMessage<V>;

/// Creates a combinator that overrides only the error code.
pub fn with_code<V>(validator: V, code: impl Into<String>) -> WithMessage<V> {
    WithMessage::code_only(validator, code)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct MinLength {
        min: usize,
    }

    impl Validate for MinLength {
        type Input = str;

        fn validate(&self, input: &str) -> Result<(), ValidationError> {
            if input.len() >= self.min {
                Ok(())
            } else {
                Err(ValidationError::new(
                    "min_length",
                    format!("Must be at least {} characters", self.min),
                ))
            }
        }
    }

    #[test]
    fn test_with_message_success() {
        let validator = WithMessage::new(MinLength { min: 3 }, "Custom message");
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_with_message_replaces_message() {
        let validator = WithMessage::new(MinLength { min: 10 }, "Password too short");
        let result = validator.validate("short");

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.message, "Password too short");
        // Original code is preserved
        assert_eq!(error.code, "min_length");
    }

    #[test]
    fn test_with_message_and_code() {
        let validator =
            WithMessage::new(MinLength { min: 10 }, "Password too short").with_code("ERR_PASSWORD");
        let result = validator.validate("short");

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.message, "Password too short");
        assert_eq!(error.code, "ERR_PASSWORD");
    }

    #[test]
    fn test_with_code_success() {
        let validator = WithMessage::code_only(MinLength { min: 3 }, "CUSTOM_CODE");
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_with_code_replaces_code() {
        let validator = WithMessage::code_only(MinLength { min: 10 }, "ERR_TOO_SHORT");
        let result = validator.validate("short");

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.code, "ERR_TOO_SHORT");
        // Original message is preserved
        assert!(error.message.contains("at least"));
    }

    #[test]
    fn test_helper_functions() {
        let v1 = with_message(MinLength { min: 3 }, "Too short");
        let v2 = with_code(MinLength { min: 3 }, "ERR");

        assert!(v1.validate("hello").is_ok());
        assert!(v2.validate("hello").is_ok());
    }

    #[test]
    fn test_nested_error_preserved() {
        let validator = WithMessage::new(MinLength { min: 10 }, "Custom");
        let result = validator.validate("short");

        let error = result.unwrap_err();
        assert_eq!(error.nested.len(), 1);
        assert_eq!(error.nested[0].code, "min_length");
    }
}
