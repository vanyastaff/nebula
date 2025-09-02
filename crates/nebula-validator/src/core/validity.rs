//! Valid and Invalid type definitions

use std::fmt::Debug;
use chrono::{DateTime, Utc};
use crate::types::{ValidationError, ValidatorId};
use super::proof::ValidationProof;

/// A valid value with proof of validation
#[derive(Debug, Clone)]
pub struct Valid<T> {
    /// The validated value
    value: T,
    /// Proof of validation
    proof: ValidationProof,
    /// Optional metadata
    metadata: ValidMetadata,
}

/// Metadata for valid values
#[derive(Debug, Clone, Default)]
pub struct ValidMetadata {
    /// When this validation expires (if applicable)
    pub expires_at: Option<DateTime<Utc>>,
    /// History of transformations applied
    pub transformations: Vec<String>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// Custom metadata
    pub custom: std::collections::HashMap<String, serde_json::Value>,
}

impl<T> Valid<T> {
    /// Create a new valid value with proof
    pub fn new(value: T, proof: ValidationProof) -> Self {
        Self {
            value,
            proof,
            metadata: ValidMetadata::default(),
        }
    }
    
    /// Create with simple proof
    pub fn with_simple_proof(value: T, validator_id: impl Into<String>) -> Self {
        Self::new(
            value,
            ValidationProof::simple(ValidatorId::new(validator_id)),
        )
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
    
    /// Get the validation proof
    pub fn proof(&self) -> &ValidationProof {
        &self.proof
    }
    
    /// Get metadata
    pub fn metadata(&self) -> &ValidMetadata {
        &self.metadata
    }
    
    /// Get mutable metadata
    pub fn metadata_mut(&mut self) -> &mut ValidMetadata {
        &mut self.metadata
    }
    
    /// Set expiration time
    pub fn with_expiration(mut self, expires_at: DateTime<Utc>) -> Self {
        self.metadata.expires_at = Some(expires_at);
        self
    }
    
    /// Add a transformation record
    pub fn with_transformation(mut self, transformation: impl Into<String>) -> Self {
        self.metadata.transformations.push(transformation.into());
        self
    }
    
    /// Add a tag
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.metadata.tags.push(tag.into());
        self
    }
    
    /// Check if the validation has expired
    pub fn is_expired(&self) -> bool {
        self.metadata.expires_at
            .map(|exp| Utc::now() > exp)
            .unwrap_or(false)
    }
    
    /// Map the value while preserving validity
    pub fn map<U, F>(self, f: F) -> Valid<U>
    where
        F: FnOnce(T) -> U,
    {
        Valid {
            value: f(self.value),
            proof: self.proof,
            metadata: self.metadata,
        }
    }
    
    /// Try to map the value, potentially invalidating it
    pub fn and_then<U, F>(self, f: F) -> Result<Valid<U>, Invalid<U>>
    where
        F: FnOnce(T) -> Result<U, Vec<ValidationError>>,
    {
        match f(self.value) {
            Ok(value) => Ok(Valid {
                value,
                proof: self.proof,
                metadata: self.metadata,
            }),
            Err(errors) => Err(Invalid::new(None, errors)),
        }
    }
    
    /// Combine two valid values
    pub fn zip<U>(self, other: Valid<U>) -> Valid<(T, U)> {
        Valid {
            value: (self.value, other.value),
            proof: self.proof.merge(other.proof),
            metadata: self.metadata, // Could merge metadata too
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
    /// Metadata about the invalid state
    metadata: InvalidMetadata,
}

/// Metadata for invalid values
#[derive(Debug, Clone, Default)]
pub struct InvalidMetadata {
    /// Number of validation attempts
    pub attempt_count: usize,
    /// Time of last validation attempt
    pub last_attempt: Option<DateTime<Utc>>,
    /// Validators that were tried
    pub tried_validators: Vec<ValidatorId>,
    /// Whether recovery was attempted
    pub recovery_attempted: bool,
    /// Custom metadata
    pub custom: std::collections::HashMap<String, serde_json::Value>,
}

impl<T> Invalid<T> {
    /// Create a new invalid value with errors
    pub fn new(value: Option<T>, errors: Vec<ValidationError>) -> Self {
        Self {
            value,
            errors,
            metadata: InvalidMetadata::default(),
        }
    }
    
    /// Create with a single error
    pub fn with_error(value: T, error: ValidationError) -> Self {
        Self::new(Some(value), vec![error])
    }
    
    /// Create without a value
    pub fn without_value(errors: Vec<ValidationError>) -> Self {
        Self::new(None, errors)
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
    
    /// Get metadata
    pub fn metadata(&self) -> &InvalidMetadata {
        &self.metadata
    }
    
    /// Get mutable metadata
    pub fn metadata_mut(&mut self) -> &mut InvalidMetadata {
        &mut self.metadata
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
    
    /// Check if a specific error code exists
    pub fn has_error_code(&self, code: &ErrorCode) -> bool {
        self.errors.iter().any(|e| e.code() == code)
    }
    
    /// Map the value if present
    pub fn map<U, F>(self, f: F) -> Invalid<U>
    where
        F: FnOnce(T) -> U,
    {
        Invalid {
            value: self.value.map(f),
            errors: self.errors,
            metadata: self.metadata,
        }
    }
    
    /// Attempt to recover from invalid state
    pub async fn try_recover<F, Fut>(mut self, recovery: F) -> Result<Valid<T>, Invalid<T>>
    where
        F: FnOnce(Option<T>, Vec<ValidationError>) -> Fut,
        Fut: std::future::Future<Output = Result<T, Vec<ValidationError>>>,
    {
        self.metadata.recovery_attempted = true;
        
        match recovery(self.value, self.errors).await {
            Ok(value) => Ok(Valid::with_simple_proof(value, "recovered")),
            Err(errors) => {
                self.value = None;
                self.errors = errors;
                Err(self)
            }
        }
    }
    
    /// Convert to a Result
    pub fn into_result(self) -> Result<T, Vec<ValidationError>> {
        match self.value {
            Some(value) => Err(self.errors),
            None => Err(self.errors),
        }
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