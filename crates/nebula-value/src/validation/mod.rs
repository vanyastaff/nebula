//! Value validation using nebula-validator

use crate::core::{Value, ValueError, ValueResult};

/// Validation trait for nebula values
pub trait ValueValidator: Send + Sync {
    /// Validate a value
    fn validate(&self, value: &Value) -> ValueResult<()>;

    /// Get validator name
    fn name(&self) -> &str;

    /// Get validator description
    fn description(&self) -> Option<&str>;
}

/// Validation result with context
pub type ValidationResult<T> = Result<T, ValidationError>;

/// Validation error (using core::error::ValidationError directly)
pub use crate::core::error::ValidationError;

// Re-export common validators
pub mod validators {
    use super::*;

    /// Text length validator
    pub struct TextLength {
        min_length: Option<usize>,
        max_length: Option<usize>,
    }

    impl Default for TextLength {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TextLength {
        /// Create a new text length validator
        pub fn new() -> Self {
            Self {
                min_length: None,
                max_length: None,
            }
        }

        /// Set minimum length
        pub fn min_length(mut self, min_length: usize) -> Self {
            self.min_length = Some(min_length);
            self
        }

        /// Set maximum length
        pub fn max_length(mut self, max_length: usize) -> Self {
            self.max_length = Some(max_length);
            self
        }
    }

    impl ValueValidator for TextLength {
        fn validate(&self, value: &Value) -> ValueResult<()> {
            if let Value::String(text) = value {
                let len = text.len();

                if let Some(min_len) = self.min_length
                    && len < min_len {
                        return Err(ValueError::Validation(ValidationError::InvalidLength {
                            actual: len,
                            constraint: format!(">= {}", min_len),
                        }));
                    }

                if let Some(max_len) = self.max_length
                    && len > max_len {
                        return Err(ValueError::Validation(ValidationError::InvalidLength {
                            actual: len,
                            constraint: format!("<= {}", max_len),
                        }));
                    }

                Ok(())
            } else {
                Err(ValueError::type_mismatch("string", value.type_name()))
            }
        }

        fn name(&self) -> &str {
            "text_length"
        }

        fn description(&self) -> Option<&str> {
            Some("Validates text length constraints")
        }
    }

    /// Number range validator
    pub struct NumberRange {
        min_value: Option<f64>,
        max_value: Option<f64>,
        integer_only: bool,
    }

    impl Default for NumberRange {
        fn default() -> Self {
            Self::new()
        }
    }

    impl NumberRange {
        /// Create a new number range validator
        pub fn new() -> Self {
            Self {
                min_value: None,
                max_value: None,
                integer_only: false,
            }
        }

        /// Set minimum value
        pub fn min_value(mut self, min_value: f64) -> Self {
            self.min_value = Some(min_value);
            self
        }

        /// Set maximum value
        pub fn max_value(mut self, max_value: f64) -> Self {
            self.max_value = Some(max_value);
            self
        }

        /// Set integer only
        pub fn integer_only(mut self, integer_only: bool) -> Self {
            self.integer_only = integer_only;
            self
        }
    }

    impl ValueValidator for NumberRange {
        fn validate(&self, value: &Value) -> ValueResult<()> {
            if self.integer_only
                && matches!(value, Value::Float(_)) {
                    return Err(ValueError::type_mismatch("integer", "float"));
                }
            let val = value
                .as_float()
                .ok_or_else(|| ValueError::type_mismatch("number", value.type_name()))?;
            if let Some(min_val) = self.min_value
                && val < min_val {
                    return Err(ValueError::Validation(ValidationError::failed(format!(
                        "Value {} is less than minimum {}",
                        val, min_val
                    ))));
                }
            if let Some(max_val) = self.max_value
                && val > max_val {
                    return Err(ValueError::Validation(ValidationError::failed(format!(
                        "Value {} is greater than maximum {}",
                        val, max_val
                    ))));
                }
            Ok(())
        }

        fn name(&self) -> &str {
            "number_range"
        }

        fn description(&self) -> Option<&str> {
            Some("Validates numeric range constraints")
        }
    }

    /// Required field validator
    pub struct Required;

    impl ValueValidator for Required {
        fn validate(&self, value: &Value) -> ValueResult<()> {
            if value.is_null() {
                Err(ValidationError::required("value").into())
            } else {
                Ok(())
            }
        }

        fn name(&self) -> &str {
            "required"
        }

        fn description(&self) -> Option<&str> {
            Some("Ensures value is not null")
        }
    }

    /// Array length validator
    pub struct ArrayLength {
        min_length: Option<usize>,
        max_length: Option<usize>,
    }

    impl Default for ArrayLength {
        fn default() -> Self {
            Self::new()
        }
    }

    impl ArrayLength {
        /// Create a new array length validator
        pub fn new() -> Self {
            Self {
                min_length: None,
                max_length: None,
            }
        }

        /// Set minimum length
        pub fn min_length(mut self, min_length: usize) -> Self {
            self.min_length = Some(min_length);
            self
        }

        /// Set maximum length
        pub fn max_length(mut self, max_length: usize) -> Self {
            self.max_length = Some(max_length);
            self
        }
    }

    impl ValueValidator for ArrayLength {
        fn validate(&self, value: &Value) -> ValueResult<()> {
            if let Value::Array(array) = value {
                let len = array.len();

                if let Some(min_len) = self.min_length
                    && len < min_len {
                        return Err(ValueError::Validation(ValidationError::InvalidLength {
                            actual: len,
                            constraint: format!(">= {}", min_len),
                        }));
                    }

                if let Some(max_len) = self.max_length
                    && len > max_len {
                        return Err(ValueError::Validation(ValidationError::InvalidLength {
                            actual: len,
                            constraint: format!("<= {}", max_len),
                        }));
                    }

                Ok(())
            } else {
                Err(ValueError::type_mismatch("array", value.type_name()))
            }
        }

        fn name(&self) -> &str {
            "array_length"
        }

        fn description(&self) -> Option<&str> {
            Some("Validates array length constraints")
        }
    }
}

// Re-export validators
pub use validators::{ArrayLength, NumberRange, Required, TextLength};

// Extension trait for Value to add validation methods
pub trait ValueValidationExt {
    /// Validate this value with a validator
    fn validate_with<V: ValueValidator>(&self, validator: &V) -> ValueResult<()>;

    /// Check if this value is valid according to a validator
    fn is_valid<V: ValueValidator>(&self, validator: &V) -> bool;
}

impl ValueValidationExt for Value {
    fn validate_with<V: ValueValidator>(&self, validator: &V) -> ValueResult<()> {
        validator.validate(self)
    }

    fn is_valid<V: ValueValidator>(&self, validator: &V) -> bool {
        validator.validate(self).is_ok()
    }
}
