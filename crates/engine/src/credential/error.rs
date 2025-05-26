use std::io;

use thiserror::Error;

use crate::types::{Key, KeyParseError};

#[derive(Error, Debug)]
pub enum CredentialError {
    // Credential identified by `key` was not found.
    #[error("Credential '{0}' is not found")]
    NotFound(Key),

    /// Build error (e.g., during configuration struct building).
    #[error("Build error: {0}")]
    BuildError(#[from] derive_builder::UninitializedFieldError),

    /// Invalid format or content for a credential key string.
    #[error("Invalid key format: {0}")]
    InvalidKeyFormat(#[from] KeyParseError),

    /// A value provided for the credential violates defined constraints.
    /// Includes the key and the reason for the violation.
    #[error("Constraint violation for credential '{key}': {reason}")]
    ConstraintViolation { key: Key, reason: String },

    /// Validation failed for a credential.
    /// Includes the key of the credential and the reason for failure.
    #[error("Validation failed for credential '{key}': {reason}")]
    ValidationError { key: Key, reason: String },

    /// Authentication error when using a credential.
    #[error("Authentication failed for credential '{key}': {reason}")]
    AuthenticationError { key: Key, reason: String },

    /// Authorization error when using a credential (e.g., insufficient
    /// permissions).
    #[error("Authorization failed for credential '{key}': {reason}")]
    AuthorizationError { key: Key, reason: String },

    /// Type mismatch or other type-related error when handling a credential's
    /// value.
    #[error("Type error for credential '{key}': Expected {expected_type}, got {actual_details}")]
    InvalidType {
        key: Key,
        expected_type: String,
        actual_details: String,
    },

    /// Credential with the specified key already exists in the collection.
    #[error("Credential with key '{0}' already exists")]
    DuplicateKey(Key),

    /// I/O error encountered while getting credentials (e.g., reading from a
    /// keychain).
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    /// External service error when using a credential.
    #[error("External service error for credential '{key}': {reason}")]
    ExternalServiceError { key: Key, reason: String },

    /// Missing required field in credential.
    #[error("Missing the required field '{field}' for credential '{key}'")]
    MissingRequiredField { key: Key, field: String },

    /// Error encrypting/decrypting credential data.
    #[error("Encryption error for credential '{key}': {reason}")]
    EncryptionError { key: Key, reason: String },

    /// Credential expired or invalid token.
    #[error("Credential '{key}' expired: {reason}")]
    Expired { key: Key, reason: String },

    /// Error deserializing or processing a credential.
    #[error("Deserialization error for credential '{key}': {error}")]
    DeserializationError { key: Key, error: String },

    /// Error serializing a credential.
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}
