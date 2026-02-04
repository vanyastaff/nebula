//! Error types for credential operations
//!
//! This module defines a three-tier error hierarchy:
//! - [`CredentialError`]: Top-level error wrapping Storage/Crypto/Validation
//! - [`StorageError`]: File I/O, permissions, not found
//! - [`CryptoError`]: Encryption, decryption, key derivation
//! - [`ValidationError`]: Invalid credential IDs, malformed data
//!
//! # Error Conversion Examples
//!
//! Errors automatically convert to [`CredentialError`] via `From` implementations:
//!
//! ```
//! use nebula_credential::core::{StorageError, CredentialError};
//!
//! // Storage errors convert automatically
//! let storage_err = StorageError::NotFound {
//!     id: "missing_cred".to_string(),
//! };
//! let cred_err: CredentialError = storage_err.into();
//! assert!(cred_err.to_string().contains("missing_cred"));
//! ```
//!
//! Using `?` operator for automatic conversion:
//!
//! ```no_run
//! use nebula_credential::core::{Result, StorageError};
//!
//! fn load_credential(id: &str) -> Result<String> {
//!     // StorageError automatically converts to CredentialError
//!     Err(StorageError::NotFound { id: id.to_string() })?
//! }
//! ```

use thiserror::Error;

/// Top-level credential error
///
/// Wraps specific error categories (storage, cryptographic, validation)
/// with contextual information for debugging and error handling.
#[derive(Debug, Error)]
pub enum CredentialError {
    /// Storage error for credential operation
    #[error("Storage error for credential '{id}': {source}")]
    Storage {
        /// Credential ID
        id: String,
        /// Underlying storage error
        #[source]
        source: StorageError,
    },

    /// Cryptographic error
    #[error("Cryptographic error: {source}")]
    Crypto {
        /// Underlying crypto error
        #[source]
        source: CryptoError,
    },

    /// Validation error
    #[error("Validation error: {source}")]
    Validation {
        /// Underlying validation error
        #[source]
        source: ValidationError,
    },
}

/// Storage operation errors
///
/// Errors related to credential persistence operations including
/// file I/O failures, permission issues, and resource not found.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Credential not found
    #[error("Credential '{id}' not found")]
    NotFound {
        /// Credential ID
        id: String,
    },

    /// Failed to read credential
    #[error("Failed to read credential '{id}': {source}")]
    ReadFailure {
        /// Credential ID
        id: String,
        /// Underlying I/O error
        #[source]
        source: std::io::Error,
    },

    /// Failed to write credential
    #[error("Failed to write credential '{id}': {source}")]
    WriteFailure {
        /// Credential ID
        id: String,
        /// Underlying I/O error
        #[source]
        source: std::io::Error,
    },

    /// Permission denied for credential operation
    #[error("Permission denied for credential '{id}'")]
    PermissionDenied {
        /// Credential ID
        id: String,
    },

    /// Operation timed out
    #[error("Operation timed out after {duration:?}")]
    Timeout {
        /// Duration attempted
        duration: std::time::Duration,
    },
}

/// Cryptographic operation errors
///
/// Errors from encryption, decryption, and key derivation operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Decryption failed - invalid key or corrupted data
    #[error("Decryption failed - invalid key or corrupted data")]
    DecryptionFailed,

    /// Encryption failed
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    /// Key derivation failed
    #[error("Key derivation failed: {0}")]
    KeyDerivation(String),

    /// Nonce generation failed
    #[error("Nonce generation failed")]
    NonceGeneration,

    /// Unsupported encryption version
    #[error("Unsupported encryption version: {0}")]
    UnsupportedVersion(u8),
}

/// Validation errors
///
/// Errors from input validation including invalid credential IDs
/// and malformed credential data.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// Credential ID cannot be empty
    #[error("Credential ID cannot be empty")]
    EmptyCredentialId,

    /// Invalid credential ID
    #[error("Invalid credential ID '{id}': {reason}")]
    InvalidCredentialId {
        /// The invalid ID
        id: String,
        /// Reason for invalidity
        reason: String,
    },

    /// Invalid credential format
    #[error("Invalid credential format: {0}")]
    InvalidFormat(String),
}

/// Result type alias for credential operations
pub type Result<T> = std::result::Result<T, CredentialError>;

// Conversion helpers for ergonomic error propagation
impl From<StorageError> for CredentialError {
    fn from(source: StorageError) -> Self {
        // Extract ID from storage error if possible
        let id = match &source {
            StorageError::NotFound { id } => id.clone(),
            StorageError::ReadFailure { id, .. } => id.clone(),
            StorageError::WriteFailure { id, .. } => id.clone(),
            StorageError::PermissionDenied { id } => id.clone(),
            StorageError::Timeout { .. } => "unknown".to_string(),
        };
        Self::Storage { id, source }
    }
}

impl From<CryptoError> for CredentialError {
    fn from(source: CryptoError) -> Self {
        Self::Crypto { source }
    }
}

impl From<ValidationError> for CredentialError {
    fn from(source: ValidationError) -> Self {
        Self::Validation { source }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn test_storage_error_not_found() {
        let err = StorageError::NotFound {
            id: "test-id".to_string(),
        };
        assert_eq!(err.to_string(), "Credential 'test-id' not found");
    }

    #[test]
    fn test_storage_error_read_failure() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err = StorageError::ReadFailure {
            id: "test-id".to_string(),
            source: io_err,
        };
        assert!(err.to_string().contains("test-id"));
        assert!(err.to_string().contains("Failed to read"));
    }

    #[test]
    fn test_crypto_error_decryption_failed() {
        let err = CryptoError::DecryptionFailed;
        assert_eq!(
            err.to_string(),
            "Decryption failed - invalid key or corrupted data"
        );
    }

    #[test]
    fn test_crypto_error_key_derivation() {
        let err = CryptoError::KeyDerivation("invalid salt".to_string());
        assert!(err.to_string().contains("Key derivation failed"));
        assert!(err.to_string().contains("invalid salt"));
    }

    #[test]
    fn test_validation_error_empty_id() {
        let err = ValidationError::EmptyCredentialId;
        assert_eq!(err.to_string(), "Credential ID cannot be empty");
    }

    #[test]
    fn test_validation_error_invalid_id() {
        let err = ValidationError::InvalidCredentialId {
            id: "../etc/passwd".to_string(),
            reason: "contains path traversal characters".to_string(),
        };
        assert!(err.to_string().contains("../etc/passwd"));
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn test_credential_error_from_storage() {
        let storage_err = StorageError::NotFound {
            id: "test-id".to_string(),
        };
        let cred_err: CredentialError = storage_err.into();
        assert!(matches!(cred_err, CredentialError::Storage { .. }));
        assert!(cred_err.to_string().contains("test-id"));
    }

    #[test]
    fn test_credential_error_from_crypto() {
        let crypto_err = CryptoError::DecryptionFailed;
        let cred_err: CredentialError = crypto_err.into();
        assert!(matches!(cred_err, CredentialError::Crypto { .. }));
        assert!(cred_err.to_string().contains("Decryption failed"));
    }

    #[test]
    fn test_credential_error_from_validation() {
        let val_err = ValidationError::EmptyCredentialId;
        let cred_err: CredentialError = val_err.into();
        assert!(matches!(cred_err, CredentialError::Validation { .. }));
        assert!(cred_err.to_string().contains("empty"));
    }

    #[test]
    fn test_error_source_chain() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
        let storage_err = StorageError::ReadFailure {
            id: "secure-cred".to_string(),
            source: io_err,
        };
        let cred_err = CredentialError::Storage {
            id: "secure-cred".to_string(),
            source: storage_err,
        };

        // Verify error chain with source()
        assert!(cred_err.source().is_some());
        let storage_source = cred_err.source().unwrap();
        assert!(storage_source.source().is_some()); // I/O error is nested
    }
}
