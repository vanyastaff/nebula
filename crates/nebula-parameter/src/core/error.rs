use nebula_error::Error;
use nebula_core::{ParameterKey, KeyParseError};
#[derive(Debug, Error)]
pub enum ParameterError {
    /// Build error (e.g., during configuration struct building).
    #[error("Build error: {0}")]
    BuildError(#[from] derive_builder::UninitializedFieldError),

    /// Invalid format or content for a parameter key string.
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(#[from] KeyParseError),

    /// Parameter identified by `key` was not found.
    #[error("Parameter '{0}' is not found")]
    NotFound(ParameterKey),

    /// Parameter with the specified key already exists in the registry.
    #[error("Parameter with a key '{0}' already exists")]
    AlreadyExists(ParameterKey),

    /// Error deserializing or processing a parameter's value.
    #[error("Deserialization error for parameter '{key}': {error}")]
    DeserializationError { key: ParameterKey, error: String },

    /// Error serializing a parameter's value.
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Type mismatch or other type-related error when handling a parameter's
    #[error("Type error for parameter '{key}': Expected {expected_type}, got {actual_details}")]
    InvalidType {
        key: ParameterKey,
        expected_type: String,
        actual_details: String,
    },

    /// Validation failed for a parameter.
    /// Includes the key of the parameter and the reason for failure.
    #[error("Validation failed for parameter '{key}': {reason}")]
    ValidationError { key: ParameterKey, reason: String },
}