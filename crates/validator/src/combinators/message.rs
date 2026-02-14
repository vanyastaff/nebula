//! MESSAGE combinator - custom error messages

use std::borrow::Cow;

use crate::core::{Validate, ValidationError, ValidatorMetadata};

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
/// use nebula_validator::core::Validate;
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

            ValidationError::new(code, self.message.clone()).with_nested_error(original)
        })
    }

    fn metadata(&self) -> ValidatorMetadata {
        let inner_meta = self.inner.metadata();

        ValidatorMetadata {
            name: format!("WithMessage({})", inner_meta.name).into(),
            description: Some(self.message.clone().into()),
            complexity: inner_meta.complexity,
            cacheable: inner_meta.cacheable,
            estimated_time: inner_meta.estimated_time,
            tags: inner_meta.tags,
            version: inner_meta.version,
            custom: inner_meta.custom,
        }
    }
}

/// Creates a WithMessage combinator.
pub fn with_message<V>(validator: V, message: impl Into<String>) -> WithMessage<V> {
    WithMessage::new(validator, message)
}

// ============================================================================
// WITH CODE COMBINATOR
// ============================================================================

/// Replaces only the error code of a validator.
///
/// Useful for categorizing errors or for i18n lookup keys.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::WithCode;
/// use nebula_validator::core::Validate;
///
/// let validator = WithCode::new(MinLength { min: 8 }, "ERR_PASSWORD_TOO_SHORT");
///
/// let result = validator.validate("short");
/// assert_eq!(result.unwrap_err().code, "ERR_PASSWORD_TOO_SHORT");
/// ```
#[derive(Debug, Clone)]
pub struct WithCode<V> {
    inner: V,
    code: String,
}

impl<V> WithCode<V> {
    /// Creates a new WithCode combinator.
    pub fn new(inner: V, code: impl Into<String>) -> Self {
        Self {
            inner,
            code: code.into(),
        }
    }

    /// Returns a reference to the inner validator.
    pub fn inner(&self) -> &V {
        &self.inner
    }

    /// Returns the custom code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Extracts the inner validator.
    pub fn into_inner(self) -> V {
        self.inner
    }
}

impl<V> Validate for WithCode<V>
where
    V: Validate,
{
    type Input = V::Input;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        self.inner.validate(input).map_err(|original| {
            ValidationError::new(self.code.clone(), original.message.clone())
                .with_nested_error(original)
        })
    }

    fn metadata(&self) -> ValidatorMetadata {
        self.inner.metadata()
    }
}

/// Creates a WithCode combinator.
pub fn with_code<V>(validator: V, code: impl Into<String>) -> WithCode<V> {
    WithCode::new(validator, code)
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
        let validator = WithCode::new(MinLength { min: 3 }, "CUSTOM_CODE");
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_with_code_replaces_code() {
        let validator = WithCode::new(MinLength { min: 10 }, "ERR_TOO_SHORT");
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
