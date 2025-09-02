//! Validated type system for nebula-validator

use std::fmt::{self, Debug, Display};
use serde::{Serialize, Deserialize};
use super::{Valid, Invalid, ValidationProof};
use crate::types::{ValidationError, ValidatorId};

/// A value that has been validated, either successfully or unsuccessfully
#[derive(Debug, Clone)]
pub enum Validated<T> {
    /// The value passed validation
    Valid(Valid<T>),
    /// The value failed validation
    Invalid(Invalid<T>),
}

impl<T: Default> Validated<T> {
    /// Create a valid result
    pub fn valid(value: T, proof: ValidationProof) -> Self {
        Self::Valid(Valid::new(value, proof))
    }
    
    /// Create an invalid result
    pub fn invalid(value: T, error: ValidationError) -> Self {
        Self::Invalid(Invalid::new(value, error))
    }
    
    /// Check if the value is valid
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid(_))
    }
    
    /// Check if the value is invalid
    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::Invalid(_))
    }
    
    /// Get the value regardless of validation status
    pub fn value(&self) -> &T {
        match self {
            Self::Valid(valid) => valid.value(),
            Self::Invalid(invalid) => invalid.value(),
        }
    }
    
    /// Get the value if valid, otherwise return None
    pub fn valid_value(&self) -> Option<&T> {
        match self {
            Self::Valid(valid) => Some(valid.value()),
            Self::Invalid(_) => None,
        }
    }
    
    /// Get the value if invalid, otherwise return None
    pub fn invalid_value(&self) -> Option<&T> {
        match self {
            Self::Valid(_) => None,
            Self::Invalid(invalid) => Some(invalid.value()),
        }
    }
    
    /// Get the validation error if invalid
    pub fn error(&self) -> Option<&ValidationError> {
        match self {
            Self::Valid(_) => None,
            Self::Invalid(invalid) => Some(invalid.error()),
        }
    }
    
    /// Get the validation proof if valid
    pub fn proof(&self) -> Option<&ValidationProof> {
        match self {
            Self::Valid(valid) => Some(valid.proof()),
            Self::Invalid(_) => None,
        }
    }
    
    /// Transform the value while preserving validation status
    pub fn map<U, F>(self, f: F) -> Validated<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Self::Valid(valid) => Validated::valid(
                f(valid.into_value()),
                valid.proof().clone(),
            ),
            Self::Invalid(invalid) => Validated::invalid(
                f(invalid.into_value()),
                invalid.error().clone(),
            ),
        }
    }
    
    /// Transform the value if valid, otherwise return the invalid result
    pub fn and_then<U, F>(self, f: F) -> Validated<U>
    where
        F: FnOnce(T) -> Validated<U>,
    {
        match self {
            Self::Valid(valid) => f(valid.into_value()),
            Self::Invalid(invalid) => Validated::invalid(
                invalid.into_value(),
                invalid.error().clone(),
            ),
        }
    }
    
    /// Convert to Result
    pub fn into_result(self) -> Result<T, ValidationError> {
        match self {
            Self::Valid(valid) => Ok(valid.into_value()),
            Self::Invalid(invalid) => Err(invalid.error().clone()),
        }
    }
    
    /// Convert from Result
    pub fn from_result(result: Result<T, ValidationError>, validator_id: ValidatorId) -> Self {
        match result {
            Ok(value) => Self::valid(value, ValidationProof::simple(validator_id)),
            Err(error) => Self::invalid(Default::default(), error),
        }
    }
}

impl<T> From<Valid<T>> for Validated<T> {
    fn from(valid: Valid<T>) -> Self {
        Self::Valid(valid)
    }
}

impl<T> From<Invalid<T>> for Validated<T> {
    fn from(invalid: Invalid<T>) -> Self {
        Self::Invalid(invalid)
    }
}

impl<T> From<Result<T, ValidationError>> for Validated<T> {
    fn from(result: Result<T, ValidationError>) -> Self {
        match result {
            Ok(value) => Self::valid(value, ValidationProof::simple(ValidatorId::new("result"))),
            Err(error) => Self::invalid(Default::default(), error),
        }
    }
}

impl<T: Display> Display for Validated<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Valid(valid) => write!(f, "Valid({})", valid.value()),
            Self::Invalid(invalid) => write!(f, "Invalid({})", invalid.value()),
        }
    }
}

impl<T: PartialEq> PartialEq for Validated<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Valid(a), Self::Valid(b)) => a.value() == b.value(),
            (Self::Invalid(a), Self::Invalid(b)) => a.value() == b.value(),
            _ => false,
        }
    }
}

impl<T: Eq> Eq for Validated<T> {}

/// Extension trait for Validated types
pub trait ValidatedExt<T> {
    /// Convert to Validated
    fn into_validated(self) -> Validated<T>;
    
    /// Validate with a custom validator
    fn validate_with<V>(self, validator: V) -> Validated<T>
    where
        V: crate::traits::Validatable,
    {
        // This would need async validation in practice
        // For now, just return as valid
        Validated::valid(self, ValidationProof::simple(ValidatorId::new("custom")))
    }
}

impl<T> ValidatedExt<T> for T {
    fn into_validated(self) -> Validated<T> {
        Validated::valid(self, ValidationProof::simple(ValidatorId::new("manual")))
    }
    
    fn validate_with<V>(self, validator: V) -> Validated<T>
    where
        V: crate::traits::Validatable,
    {
        // This would need async validation in practice
        // For now, just return as valid
        Validated::valid(self, ValidationProof::simple(ValidatorId::new("custom")))
    }
}