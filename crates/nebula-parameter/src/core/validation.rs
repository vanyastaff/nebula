//! Parameter validation using nebula-validator
//!
//! # Examples
//!
//! ```ignore
//! use nebula_parameter::core::ParameterValidation;
//! use nebula_validator::validators::string::{min_length, email};
//! use nebula_validator::combinators::and;
//!
//! let validation = ParameterValidation::from(and(min_length(3), email()));
//! ```

use nebula_validator::core::{AsValidatable, ValidationError, Validator};
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::sync::Arc;

type ValidateFn = dyn Fn(&Value) -> Result<(), ValidationError> + Send + Sync;

/// Validation configuration for parameters.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ParameterValidation {
    #[serde(skip)]
    validate_fn: Option<Arc<ValidateFn>>,

    message: Option<String>,
}

impl std::fmt::Debug for ParameterValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterValidation")
            .field("has_validator", &self.validate_fn.is_some())
            .field("message", &self.message)
            .finish()
    }
}

impl ParameterValidation {
    /// Create validation from any validator.
    pub fn from<V, T>(validator: V) -> Self
    where
        V: Validator<Input = T> + Send + Sync + 'static,
        T: ?Sized + 'static,
        Value: AsValidatable<T>,
        for<'a> <Value as AsValidatable<T>>::Output<'a>: Borrow<T>,
    {
        Self {
            validate_fn: Some(Arc::new(move |value| validator.validate_any(value))),
            message: None,
        }
    }

    /// Set custom validation message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Get the custom validation message.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Validate a value (skips null).
    pub fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        if value.is_null() {
            return Ok(());
        }

        if let Some(f) = &self.validate_fn {
            if let Err(mut err) = f(value) {
                if let Some(msg) = &self.message {
                    err = ValidationError::new(&err.code, msg);
                }
                return Err(err);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_validator::combinators::and;
    use nebula_validator::validators::string::{email, max_length, min_length};

    #[test]
    fn test_string_validation() {
        let validation = ParameterValidation::from(and(min_length(3), max_length(10)));

        assert!(validation.validate(&Value::text("hello")).is_ok());
        assert!(validation.validate(&Value::text("hi")).is_err());
        assert!(validation.validate(&Value::text("hello world!")).is_err());
    }

    #[test]
    fn test_email_validation() {
        let validation = ParameterValidation::from(email());

        assert!(
            validation
                .validate(&Value::text("user@example.com"))
                .is_ok()
        );
        assert!(validation.validate(&Value::text("not-an-email")).is_err());
    }

    #[test]
    fn test_number_validation() {
        use nebula_validator::validators::numeric::{max, min};
        let validation = ParameterValidation::from(and(min(0.0), max(100.0)));

        assert!(validation.validate(&Value::float(50.0)).is_ok());
        assert!(validation.validate(&Value::float(-10.0)).is_err());
        assert!(validation.validate(&Value::float(150.0)).is_err());
    }

    #[test]
    fn test_null_skipped() {
        let validation = ParameterValidation::from(min_length(3));
        assert!(validation.validate(&Value::Null).is_ok());
    }

    #[test]
    fn test_type_mismatch() {
        let validation = ParameterValidation::from(min_length(3));

        assert!(validation.validate(&Value::text("hello")).is_ok());

        let err = validation.validate(&Value::integer(42));
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().code, "type_mismatch");
    }
}
