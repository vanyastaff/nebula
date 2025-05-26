use std::io;

use thiserror::Error;

use crate::parameter::collection::ParameterCollectionError;
use crate::parameter::condition::ParameterCheckError;
use crate::types::{Key, KeyParseError};

/// Error types for parameter operations
#[derive(Debug, Error)]
pub enum ParameterError {
    /// Parameter identified by `key` was not found.
    #[error("Parameter '{0}' is not found")]
    NotFound(Key),

    /// Parameter with the specified key already exists in the registry.
    #[error("Parameter with a key '{0}' already exists")]
    AlreadyExists(Key),

    /// Build error (e.g., during configuration struct building).
    #[error("Build error: {0}")]
    BuildError(#[from] derive_builder::UninitializedFieldError),

    /// Invalid format or content for a parameter key string.
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(#[from] KeyParseError),

    /// A value provided for the parameter violates defined constraints.
    /// Includes the key and the reason for the violation.
    #[error("Constraint violation for parameter '{key}': {reason}")]
    ConstraintViolation { key: Key, reason: String },

    /// Validation failed for a parameter.
    /// Includes the key of the parameter and the reason for failure.
    #[error("Validation failed for parameter '{key}': {reason}")]
    ValidationError { key: Key, reason: String },

    /// Multiple validation errors occurred
    #[error("Multiple validation errors occurred ({0:?} errors)")]
    ValidationErrors(Vec<ParameterCheckError>),

    /// Error deserializing or processing a parameter's value.
    #[error("Deserialization error for parameter '{key}': {error}")]
    DeserializationError { key: Key, error: String },

    /// Error serializing a parameter's value.
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Type mismatch or other type-related error when handling a parameter's
    /// value. Includes the parameter key, the expected type, and details
    /// about the actual type or error.
    #[error("Type error for parameter '{key}': Expected {expected_type}, got {actual_details}")]
    InvalidType {
        key: Key,
        expected_type: String,
        actual_details: String,
    },

    /// I/O error encountered while getting parameters (e.g., reading from a
    /// file).
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),
}

impl PartialEq for ParameterError {
    fn eq(&self, other: &Self) -> bool {
        use ParameterError::*;
        match (self, other) {
            (NotFound(a), NotFound(b)) => a == b,

            (BuildError(e1), BuildError(e2)) => e1.field_name() == e2.field_name(),

            (InvalidKeyFormat(e1), InvalidKeyFormat(e2)) => e1 == e2,

            (
                ConstraintViolation {
                    key: k1,
                    reason: r1,
                },
                ConstraintViolation {
                    key: k2,
                    reason: r2,
                },
            ) => k1 == k2 && r1 == r2,

            (
                ValidationError {
                    key: k1,
                    reason: r1,
                },
                ValidationError {
                    key: k2,
                    reason: r2,
                },
            ) => k1 == k2 && r1 == r2,

            (ValidationErrors(e1), ValidationErrors(e2)) => {
                e1.iter().zip(e2.iter()).all(|(a, b)| a == b)
            }

            (
                DeserializationError { key: k1, error: e1 },
                DeserializationError { key: k2, error: e2 },
            ) => k1 == k2 && e1 == e2,
            (SerializationError(e1), SerializationError(e2)) => e1.to_string() == e2.to_string(),

            (
                InvalidType {
                    key: k1,
                    expected_type: et1,
                    actual_details: ad1,
                },
                InvalidType {
                    key: k2,
                    expected_type: et2,
                    actual_details: ad2,
                },
            ) => k1 == k2 && et1 == et2 && ad1 == ad2,

            (IoError(e1), IoError(e2)) => {
                e1.kind() == e2.kind() && e1.to_string() == e2.to_string()
            }

            _ => false,
        }
    }
}

impl Eq for ParameterError {}

impl From<ParameterCollectionError> for ParameterError {
    fn from(error: ParameterCollectionError) -> Self {
        match error {
            ParameterCollectionError::DuplicateKey(key) => ParameterError::AlreadyExists(key),

            ParameterCollectionError::NotFound(key) => ParameterError::NotFound(key),

            ParameterCollectionError::KeyError(key_error) => {
                ParameterError::InvalidKeyFormat(key_error)
            }

            ParameterCollectionError::ParameterError(param_error) => param_error,

            ParameterCollectionError::TypeError { key, expected } => ParameterError::InvalidType {
                key,
                expected_type: expected,
                actual_details: "Wrong parameter type".to_string(),
            },
        }
    }
}
