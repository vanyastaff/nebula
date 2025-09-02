//! Value validation using nebula-validator

use crate::core::{Value, ValueResult, ValueError};
use crate::types::*;

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

/// Validation error
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Validation failed: {message}")]
    Failed {
        message: String,
        field_path: Option<String>,
        actual_value: Option<Value>,
        expected_value: Option<Value>,
    },
    
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch {
        expected: String,
        actual: String,
        field_path: Option<String>,
    },
    
    #[error("Value out of range: {message}")]
    OutOfRange {
        message: String,
        field_path: Option<String>,
        min: Option<Value>,
        max: Option<Value>,
        actual: Value,
    },
    
    #[error("Required field missing: {field}")]
    MissingField {
        field: String,
    },
    
    #[error("Invalid format: {message}")]
    InvalidFormat {
        message: String,
        field_path: Option<String>,
        value: Value,
    },
}

impl ValidationError {
    /// Create a validation failed error
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
            field_path: None,
            actual_value: None,
            expected_value: None,
        }
    }
    
    /// Create a type mismatch error
    pub fn type_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::TypeMismatch {
            expected: expected.into(),
            actual: actual.into(),
            field_path: None,
        }
    }
    
    /// Create an out of range error
    pub fn out_of_range(message: impl Into<String>, actual: Value) -> Self {
        Self::OutOfRange {
            message: message.into(),
            field_path: None,
            min: None,
            max: None,
            actual,
        }
    }
    
    /// Create a missing field error
    pub fn missing_field(field: impl Into<String>) -> Self {
        Self::MissingField {
            field: field.into(),
        }
    }
    
    /// Create an invalid format error
    pub fn invalid_format(message: impl Into<String>, value: Value) -> Self {
        Self::InvalidFormat {
            message: message.into(),
            field_path: None,
            value,
        }
    }
    
    /// Set field path
    pub fn with_field_path(mut self, field_path: impl Into<String>) -> Self {
        match &mut self {
            Self::Failed { field_path: fp, .. } => *fp = Some(field_path.into()),
            Self::TypeMismatch { field_path: fp, .. } => *fp = Some(field_path.into()),
            Self::OutOfRange { field_path: fp, .. } => *fp = Some(field_path.into()),
            Self::InvalidFormat { field_path: fp, .. } => *fp = Some(field_path.into()),
            Self::MissingField { .. } => {}, // No field path for missing field
        }
        self
    }
    
    /// Set actual value
    pub fn with_actual_value(mut self, actual_value: Value) -> Self {
        if let Self::Failed { actual_value: av, .. } = &mut self {
            *av = Some(actual_value);
        }
        self
    }
    
    /// Set expected value
    pub fn with_expected_value(mut self, expected_value: Value) -> Self {
        if let Self::Failed { expected_value: ev, .. } = &mut self {
            *ev = Some(expected_value);
        }
        self
    }
    
    /// Set range values
    pub fn with_range(mut self, min: Option<Value>, max: Option<Value>) -> Self {
        if let Self::OutOfRange { min: min_val, max: max_val, .. } = &mut self {
            *min_val = min;
            *max_val = max;
        }
        self
    }
}

// Convert ValidationError to ValueError for integration
impl From<ValidationError> for ValueError {
    fn from(err: ValidationError) -> Self {
        ValueError::Validation(err.to_string())
    }
}

// Re-export common validators
pub mod validators {
    use super::*;
    
    /// Text length validator
    pub struct TextLength {
        min_length: Option<usize>,
        max_length: Option<usize>,
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
                
                if let Some(min_len) = self.min_length {
                    if len < min_len {
                        return Err(ValidationError::out_of_range(
                            format!("Text length {} is less than minimum {}", len, min_len),
                            value.clone()
                        ).with_range(Some(Value::int(min_len as i64)), None));
                    }
                }
                
                if let Some(max_len) = self.max_length {
                    if len > max_len {
                        return Err(ValidationError::out_of_range(
                            format!("Text length {} is greater than maximum {}", len, max_len),
                            value.clone()
                        ).with_range(None, Some(Value::int(max_len as i64))));
                    }
                }
                
                Ok(())
            } else {
                Err(ValidationError::type_mismatch("string", value.type_name()))
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
            match value {
                Value::Int(int_val) => {
                    let val = int_val.as_f64();
                    self.validate_numeric(val, value)?;
                }
                Value::Float(float_val) => {
                    let val = float_val.as_f64();
                    if self.integer_only {
                        return Err(ValidationError::type_mismatch("integer", "float"));
                    }
                    self.validate_numeric(val, value)?;
                }
                _ => {
                    return Err(ValidationError::type_mismatch("number", value.type_name()));
                }
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
    
    impl NumberRange {
        fn validate_numeric(&self, val: f64, original_value: &Value) -> ValueResult<()> {
            if let Some(min_val) = self.min_value {
                if val < min_val {
                    return Err(ValidationError::out_of_range(
                        format!("Value {} is less than minimum {}", val, min_val),
                        original_value.clone()
                    ).with_range(Some(Value::float(min_val)), None));
                }
            }
            
            if let Some(max_val) = self.max_value {
                if val > max_val {
                    return Err(ValidationError::out_of_range(
                        format!("Value {} is greater than maximum {}", val, max_val),
                        original_value.clone()
                    ).with_range(None, Some(Value::float(max_val))));
                }
            }
            
            Ok(())
        }
    }
    
    /// Required field validator
    pub struct Required;
    
    impl ValueValidator for Required {
        fn validate(&self, value: &Value) -> ValueResult<()> {
            if value.is_null() {
                Err(ValidationError::missing_field("value"))
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
                
                if let Some(min_len) = self.min_length {
                    if len < min_len {
                        return Err(ValidationError::out_of_range(
                            format!("Array length {} is less than minimum {}", len, min_len),
                            value.clone()
                        ).with_range(Some(Value::int(min_len as i64)), None));
                    }
                }
                
                if let Some(max_len) = self.max_length {
                    if len > max_len {
                        return Err(ValidationError::out_of_range(
                            format!("Array length {} is greater than maximum {}", len, max_len),
                            value.clone()
                        ).with_range(None, Some(Value::int(max_len as i64))));
                    }
                }
                
                Ok(())
            } else {
                Err(ValidationError::type_mismatch("array", value.type_name()))
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
pub use validators::{TextLength, NumberRange, Required, ArrayLength};

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
