//! Parameter validation using nebula-validator
//!
//! This module provides integration between `nebula-validator` and parameter values.
//!
//! # Examples
//!
//! ```ignore
//! use nebula_parameter::core::ParameterValidation;
//! use nebula_validator::validators::string::{min_length, max_length, email};
//! use nebula_validator::validators::numeric::{min, max, positive};
//! use nebula_validator::combinators::and;
//!
//! // String validation
//! let validation = ParameterValidation::from(and(min_length(3), max_length(50)));
//!
//! // Email validation
//! let validation = ParameterValidation::from(email());
//!
//! // Number range
//! let validation = ParameterValidation::from(and(min(18.0), max(120.0)));
//! ```

use nebula_validator::core::{AsValidatable, ValidationError, Validator};
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::sync::Arc;

/// Type-erased validator that works with Value.
trait ValueValidator: Send + Sync {
    fn validate_value(&self, value: &Value) -> Result<(), ValidationError>;
}

/// Wrapper to store any Validator<Input=T> as ValueValidator.
struct ValidatorWrapper<V, T: ?Sized + 'static> {
    validator: V,
    _phantom: std::marker::PhantomData<fn() -> T>,
}

impl<V, T> ValueValidator for ValidatorWrapper<V, T>
where
    V: Validator<Input = T> + Send + Sync,
    T: ?Sized + 'static,
    Value: AsValidatable<T>,
    for<'a> <Value as AsValidatable<T>>::Output<'a>: Borrow<T>,
{
    fn validate_value(&self, value: &Value) -> Result<(), ValidationError> {
        self.validator.validate_any(value)
    }
}

// =============================================================================
// ParameterValidation
// =============================================================================

/// Validation configuration for parameters.
///
/// Wraps validators from `nebula-validator` with custom error messages.
/// Note: Required field checking is handled by `ParameterMetadata.required`.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ParameterValidation {
    /// The underlying validator (type-erased)
    #[serde(skip)]
    validator: Option<Arc<dyn ValueValidator>>,

    /// Custom validation message override
    message: Option<String>,
}

impl std::fmt::Debug for ParameterValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterValidation")
            .field("has_validator", &self.validator.is_some())
            .field("message", &self.message)
            .finish()
    }
}

impl ParameterValidation {
    /// Create validation from any validator.
    ///
    /// The type is automatically extracted from `Value` using `AsValidatable`.
    /// If the value type doesn't match, validation returns a type mismatch error.
    pub fn from<V, T>(validator: V) -> Self
    where
        V: Validator<Input = T> + Send + Sync + 'static,
        T: ?Sized + 'static,
        Value: AsValidatable<T>,
        for<'a> <Value as AsValidatable<T>>::Output<'a>: Borrow<T>,
    {
        Self {
            validator: Some(Arc::new(ValidatorWrapper {
                validator,
                _phantom: std::marker::PhantomData,
            })),
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

    /// Validate a value (skips null values).
    pub fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        // Skip validation for null values (required check is in ParameterMetadata)
        if value.is_null() {
            return Ok(());
        }

        // Run validator if present
        if let Some(validator) = &self.validator {
            let result = validator.validate_value(value);

            if let Err(mut err) = result {
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

        // Null values are skipped (required check is elsewhere)
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
