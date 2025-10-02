//! Valid and Invalid type definitions

use super::error::ValidationError;
use std::fmt::{self, Debug, Display};

/// A valid value
#[derive(Debug, Clone)]
pub struct Valid<T> {
    /// The validated value
    value: T,
    /// Validator that validated this value
    validator_name: String,
}

impl<T> Valid<T> {
    /// Create a new valid value
    pub fn new(value: T, validator_name: impl Into<String>) -> Self {
        Self {
            value,
            validator_name: validator_name.into(),
        }
    }

    /// Create a simple valid result
    pub fn simple(value: T) -> Self {
        Self::new(value, "simple")
    }

    /// Get a reference to the value
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Get a mutable reference to the value
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Consume and return the value
    pub fn into_value(self) -> T {
        self.value
    }

    /// Get the validator name
    pub fn validator_name(&self) -> &str {
        &self.validator_name
    }

    /// Map the value while preserving validity
    pub fn map<U, F>(self, f: F) -> Valid<U>
    where
        F: FnOnce(T) -> U,
    {
        Valid {
            value: f(self.value),
            validator_name: self.validator_name,
        }
    }
}

impl<T: Display> Display for Valid<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Valid({})", self.value)
    }
}

// ==================== Invalid Type ====================

/// An invalid value with validation errors
#[derive(Debug, Clone)]
pub struct Invalid<T> {
    /// The invalid value (if available)
    value: Option<T>,
    /// Validation errors
    errors: Vec<ValidationError>,
}

impl<T> Invalid<T> {
    /// Create a new invalid value with errors
    pub fn new(value: Option<T>, errors: Vec<ValidationError>) -> Self {
        Self { value, errors }
    }

    /// Create with a single error
    pub fn with_error(value: T, error: ValidationError) -> Self {
        Self::new(Some(value), vec![error])
    }

    /// Create without a value
    pub fn without_value(errors: Vec<ValidationError>) -> Self {
        Self::new(None, errors)
    }

    /// Create a simple invalid result with a single error message
    pub fn simple(message: impl Into<String>) -> Self {
        Self::without_value(vec![ValidationError::new(message)])
    }

    /// Get the value if available
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Get mutable value if available
    pub fn value_mut(&mut self) -> Option<&mut T> {
        self.value.as_mut()
    }

    /// Take the value
    pub fn into_value(self) -> Option<T> {
        self.value
    }

    /// Get the errors
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Get mutable errors
    pub fn errors_mut(&mut self) -> &mut Vec<ValidationError> {
        &mut self.errors
    }

    /// Add an error
    pub fn add_error(mut self, error: ValidationError) -> Self {
        self.errors.push(error);
        self
    }

    /// Add multiple errors
    pub fn add_errors(mut self, errors: impl IntoIterator<Item = ValidationError>) -> Self {
        self.errors.extend(errors);
        self
    }

    /// Get the first error
    pub fn first_error(&self) -> Option<&ValidationError> {
        self.errors.first()
    }

    /// Check if a specific error message exists
    pub fn has_error_message(&self, message: &str) -> bool {
        self.errors.iter().any(|e| e.message.contains(message))
    }

    /// Map the value if present
    pub fn map<U, F>(self, f: F) -> Invalid<U>
    where
        F: FnOnce(T) -> U,
    {
        Invalid {
            value: self.value.map(f),
            errors: self.errors,
        }
    }

    /// Convert to a Result
    pub fn into_result(self) -> Result<T, Vec<ValidationError>> {
        match self.value {
            Some(_value) => Err(self.errors),
            None => Err(self.errors),
        }
    }

    /// Add context to all error messages
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        let context = context.into();
        for error in &mut self.errors {
            error.message = format!("{}: {}", context, error.message);
        }
        self
    }

    /// Add validator name to all errors
    pub fn with_validator_name(mut self, validator_name: impl Into<String>) -> Self {
        let validator_name = validator_name.into();
        for error in &mut self.errors {
            if error.validator.is_none() {
                error.validator = Some(validator_name.clone());
            }
        }
        self
    }

    /// Add field path to all errors
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        let path = path.into();
        for error in &mut self.errors {
            if error.path.is_none() {
                error.path = Some(path.clone());
            }
        }
        self
    }

    /// Combine with another Invalid, merging errors
    pub fn combine(mut self, other: Invalid<T>) -> Self {
        self.errors.extend(other.errors);
        self
    }
}

impl<T> Display for Invalid<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid({} errors)", self.errors.len())
    }
}

impl<T> From<ValidationError> for Invalid<T> {
    fn from(error: ValidationError) -> Self {
        Invalid::without_value(vec![error])
    }
}

impl<T> From<Vec<ValidationError>> for Invalid<T> {
    fn from(errors: Vec<ValidationError>) -> Self {
        Invalid::without_value(errors)
    }
}
