//! Validation result types

use serde::{Serialize, Deserialize};
use std::fmt::{self, Display};
use super::{ValidationError, ValidationMetadata, ValidationId};

/// Result of a validation operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationResult<T> {
    /// Validation succeeded
    Ok(T),
    /// Validation failed with errors
    Err(Vec<ValidationError>),
}

impl<T> ValidationResult<T> {
    /// Create a successful result
    pub fn success(value: T) -> Self {
        ValidationResult::Ok(value)
    }
    
    /// Create a failed result with errors
    pub fn failure(errors: Vec<ValidationError>) -> Self {
        ValidationResult::Err(errors)
    }
    
    /// Create a failed result with a single error
    pub fn error(error: ValidationError) -> Self {
        ValidationResult::Err(vec![error])
    }
    
    /// Check if the result is successful
    pub fn is_success(&self) -> bool {
        matches!(self, ValidationResult::Ok(_))
    }
    
    /// Check if the result is a failure
    pub fn is_failure(&self) -> bool {
        matches!(self, ValidationResult::Err(_))
    }
    
    /// Get the success value if present
    pub fn ok(self) -> Option<T> {
        match self {
            ValidationResult::Ok(value) => Some(value),
            ValidationResult::Err(_) => None,
        }
    }
    
    /// Get the errors if present
    pub fn err(self) -> Option<Vec<ValidationError>> {
        match self {
            ValidationResult::Ok(_) => None,
            ValidationResult::Err(errors) => Some(errors),
        }
    }
    
    /// Convert to a standard Result
    pub fn into_result(self) -> Result<T, Vec<ValidationError>> {
        match self {
            ValidationResult::Ok(value) => Ok(value),
            ValidationResult::Err(errors) => Err(errors),
        }
    }
    
    /// Map the success value
    pub fn map<U, F>(self, f: F) -> ValidationResult<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            ValidationResult::Ok(value) => ValidationResult::Ok(f(value)),
            ValidationResult::Err(errors) => ValidationResult::Err(errors),
        }
    }
    
    /// Map the error value
    pub fn map_err<F>(self, f: F) -> ValidationResult<T>
    where
        F: FnOnce(Vec<ValidationError>) -> Vec<ValidationError>,
    {
        match self {
            ValidationResult::Ok(value) => ValidationResult::Ok(value),
            ValidationResult::Err(errors) => ValidationResult::Err(f(errors)),
        }
    }
    
    /// Chain another validation
    pub fn and_then<U, F>(self, f: F) -> ValidationResult<U>
    where
        F: FnOnce(T) -> ValidationResult<U>,
    {
        match self {
            ValidationResult::Ok(value) => f(value),
            ValidationResult::Err(errors) => ValidationResult::Err(errors),
        }
    }
    
    /// Provide an alternative result on failure
    pub fn or_else<F>(self, f: F) -> ValidationResult<T>
    where
        F: FnOnce(Vec<ValidationError>) -> ValidationResult<T>,
    {
        match self {
            ValidationResult::Ok(value) => ValidationResult::Ok(value),
            ValidationResult::Err(errors) => f(errors),
        }
    }
    
    /// Get a reference to the success value
    pub fn as_ref(&self) -> ValidationResult<&T> {
        match self {
            ValidationResult::Ok(value) => ValidationResult::Ok(value),
            ValidationResult::Err(errors) => ValidationResult::Err(errors.clone()),
        }
    }
    
    /// Unwrap the success value or panic
    pub fn unwrap(self) -> T {
        match self {
            ValidationResult::Ok(value) => value,
            ValidationResult::Err(errors) => {
                panic!("Validation failed with {} errors", errors.len())
            }
        }
    }
    
    /// Unwrap the success value or use a default
    pub fn unwrap_or(self, default: T) -> T {
        match self {
            ValidationResult::Ok(value) => value,
            ValidationResult::Err(_) => default,
        }
    }
}

impl<T: Default> ValidationResult<T> {
    /// Get the success value or the default
    pub fn unwrap_or_default(self) -> T {
        self.unwrap_or(T::default())
    }
}

impl<T: Display> Display for ValidationResult<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationResult::Ok(value) => write!(f, "Ok({})", value),
            ValidationResult::Err(errors) => {
                write!(f, "Err({} validation errors)", errors.len())
            }
        }
    }
}

impl<T> From<Result<T, ValidationError>> for ValidationResult<T> {
    fn from(result: Result<T, ValidationError>) -> Self {
        match result {
            Ok(value) => ValidationResult::Ok(value),
            Err(error) => ValidationResult::Err(vec![error]),
        }
    }
}

impl<T> From<Result<T, Vec<ValidationError>>> for ValidationResult<T> {
    fn from(result: Result<T, Vec<ValidationError>>) -> Self {
        match result {
            Ok(value) => ValidationResult::Ok(value),
            Err(errors) => ValidationResult::Err(errors),
        }
    }
}

/// Result of batch validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchValidationResult<T> {
    /// Successfully validated items
    pub valid: Vec<(usize, T)>,
    /// Failed items with their errors
    pub invalid: Vec<(usize, Vec<ValidationError>)>,
    /// Total items processed
    pub total: usize,
    /// Metadata about the batch validation
    pub metadata: ValidationMetadata,
}

impl<T> BatchValidationResult<T> {
    /// Create a new batch result
    pub fn new() -> Self {
        Self {
            valid: Vec::new(),
            invalid: Vec::new(),
            total: 0,
            metadata: ValidationMetadata::default(),
        }
    }
    
    /// Add a valid item
    pub fn add_valid(&mut self, index: usize, value: T) {
        self.valid.push((index, value));
        self.total += 1;
    }
    
    /// Add an invalid item
    pub fn add_invalid(&mut self, index: usize, errors: Vec<ValidationError>) {
        self.invalid.push((index, errors));
        self.total += 1;
    }
    
    /// Check if all items are valid
    pub fn all_valid(&self) -> bool {
        self.invalid.is_empty()
    }
    
    /// Get the success rate
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.valid.len() as f64 / self.total as f64
        }
    }
}

impl<T> Default for BatchValidationResult<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of stream validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamValidationResult {
    /// Number of items processed
    pub items_processed: usize,
    /// Number of successful validations
    pub items_valid: usize,
    /// Number of failed validations
    pub items_invalid: usize,
    /// Errors encountered (limited to prevent memory issues)
    pub errors: Vec<ValidationError>,
    /// Maximum errors to store
    pub max_errors: usize,
    /// Stream metadata
    pub metadata: ValidationMetadata,
}

impl StreamValidationResult {
    /// Create a new stream result
    pub fn new(max_errors: usize) -> Self {
        Self {
            items_processed: 0,
            items_valid: 0,
            items_invalid: 0,
            errors: Vec::new(),
            max_errors,
            metadata: ValidationMetadata::default(),
        }
    }
    
    /// Record a valid item
    pub fn record_valid(&mut self) {
        self.items_processed += 1;
        self.items_valid += 1;
    }
    
    /// Record an invalid item
    pub fn record_invalid(&mut self, errors: Vec<ValidationError>) {
        self.items_processed += 1;
        self.items_invalid += 1;
        
        // Only store errors up to the limit
        let remaining = self.max_errors.saturating_sub(self.errors.len());
        self.errors.extend(errors.into_iter().take(remaining));
    }
    
    /// Get the success rate
    pub fn success_rate(&self) -> f64 {
        if self.items_processed == 0 {
            0.0
        } else {
            self.items_valid as f64 / self.items_processed as f64
        }
    }
}