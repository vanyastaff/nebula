//! Error types for credential operations
//!
//! This module defines a layered error hierarchy:
//! - [`CredentialError`]: Top-level error wrapping Crypto/Validation
//! - [`CryptoError`]: Encryption, decryption, key derivation
//! - [`ValidationError`]: Invalid credential IDs, malformed data
//! - [`StoreError`](crate::StoreError): Storage-layer errors (not found, conflict)
//!
//! # Error Conversion Examples
//!
//! [`CryptoError`] and [`ValidationError`] convert to [`CredentialError`] via
//! `From` implementations:
//!
//! ```
//! use nebula_credential::error::{CredentialError, ValidationError};
//!
//! // Validation errors convert automatically
//! let val_err = ValidationError::InvalidCredentialId {
//!     id: "bad id".to_string(),
//!     reason: "contains spaces".to_string(),
//! };
//! let cred_err: CredentialError = val_err.into();
//! assert!(cred_err.to_string().contains("bad id"));
//! ```
//!
//! [`StoreError`](crate::StoreError) is used directly by the storage layer:
//!
//! ```
//! use nebula_credential::StoreError;
//!
//! let err = StoreError::NotFound { id: "missing_cred".to_string() };
//! assert!(err.to_string().contains("missing_cred"));
//! ```

use thiserror::Error;

/// Top-level credential error
///
/// Wraps specific error categories (storage, cryptographic, validation)
/// with contextual information for debugging and error handling.
#[derive(Debug, Error)]
pub enum CredentialError {
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

    /// Operation requires an interactive credential, but this credential
    /// is non-interactive (v2).
    #[error("Credential does not support interactive flows")]
    NotInteractive,

    /// Provider-specific error from a credential implementation (v2).
    #[error("Provider error: {0}")]
    Provider(String),

    /// Invalid input from user (parameter values).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Refresh failed with structured error info.
    #[error("refresh failed ({kind:?}): {source}")]
    RefreshFailed {
        /// What kind of refresh failure.
        kind: RefreshErrorKind,
        /// Retry guidance for the framework.
        retry: RetryAdvice,
        /// Underlying cause.
        #[source]
        source: Box<dyn std::error::Error + Send + 'static>,
    },

    /// Credential revocation failed.
    #[error("revoke failed: {source}")]
    RevokeFailed {
        /// Underlying cause.
        #[source]
        source: Box<dyn std::error::Error + Send + 'static>,
    },

    /// Credential composition not available (no resolver in context).
    #[error("credential composition not available")]
    CompositionNotAvailable,

    /// Composed credential resolution failed.
    #[error("composition failed: {source}")]
    CompositionFailed {
        /// Underlying cause.
        #[source]
        source: Box<dyn std::error::Error + Send + 'static>,
    },

    /// Scheme type mismatch between credential and resource.
    #[error("scheme mismatch: expected {expected}, got {actual}")]
    SchemeMismatch {
        /// Expected scheme kind.
        expected: &'static str,
        /// Actual scheme kind found.
        actual: String,
    },
}

/// Cryptographic operation errors
///
/// Errors from encryption, decryption, and key derivation operations.
#[derive(Debug, Error, nebula_error::Classify)]
pub enum CryptoError {
    /// Decryption failed - invalid key or corrupted data
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_DECRYPT")]
    #[error("Decryption failed - invalid key or corrupted data")]
    DecryptionFailed,

    /// Encryption failed
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_ENCRYPT")]
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    /// Key derivation failed
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_KEY")]
    #[error("Key derivation failed: {0}")]
    KeyDerivation(String),

    /// Nonce generation failed
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_NONCE")]
    #[error("Nonce generation failed")]
    NonceGeneration,

    /// Unsupported encryption version
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_VERSION")]
    #[error("Unsupported encryption version: {0}")]
    UnsupportedVersion(u8),
}

/// Validation errors
///
/// Errors from input validation including invalid credential IDs
/// and malformed credential data.
#[derive(Debug, Error, nebula_error::Classify)]
pub enum ValidationError {
    /// Credential ID cannot be empty
    #[classify(category = "validation", code = "CREDENTIAL:EMPTY_ID")]
    #[error("Credential ID cannot be empty")]
    EmptyCredentialId,

    /// Invalid credential ID
    #[classify(category = "validation", code = "CREDENTIAL:INVALID_ID")]
    #[error("Invalid credential ID '{id}': {reason}")]
    InvalidCredentialId {
        /// The invalid ID
        id: String,
        /// Reason for invalidity
        reason: String,
    },

    /// Invalid credential format
    #[classify(category = "validation", code = "CREDENTIAL:INVALID_FORMAT")]
    #[error("Invalid credential format: {0}")]
    InvalidFormat(String),
}

/// What kind of refresh failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshErrorKind {
    /// Refresh token itself has expired -- needs re-authentication.
    TokenExpired,
    /// Credential was explicitly revoked at the provider.
    TokenRevoked,
    /// Transient network error -- retry may succeed.
    TransientNetwork,
    /// Provider is temporarily unavailable.
    ProviderUnavailable,
    /// Protocol-level error (invalid grant, bad response format).
    ProtocolError,
}

/// Retry guidance from credential to framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryAdvice {
    /// Never retry -- permanent failure.
    Never,
    /// Retry immediately.
    Immediate,
    /// Retry after the given duration.
    After(std::time::Duration),
}

/// Where in the resolution pipeline an error occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolutionStage {
    /// Loading state from store.
    LoadState,
    /// Decrypting stored data.
    Decrypt,
    /// Deserializing state bytes.
    DeserializeState,
    /// Projecting scheme from state.
    ProjectScheme,
    /// Coercing scheme to resource Auth type.
    CoerceToResourceAuth,
    /// Refreshing expired credentials.
    Refresh,
}

/// Simple string-based error for convenience constructors.
#[derive(Debug)]
struct SimpleError(String);

impl std::fmt::Display for SimpleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SimpleError {}

impl CredentialError {
    /// Shorthand for `RefreshFailed` with a string message.
    pub fn refresh(
        kind: RefreshErrorKind,
        retry: RetryAdvice,
        msg: impl std::fmt::Display,
    ) -> Self {
        Self::RefreshFailed {
            kind,
            retry,
            source: Box::new(SimpleError(msg.to_string())),
        }
    }
}

impl nebula_error::Classify for CredentialError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Crypto { source } => nebula_error::Classify::category(source),
            Self::Validation { source } => nebula_error::Classify::category(source),
            Self::NotInteractive => nebula_error::ErrorCategory::Unsupported,
            Self::Provider(_) => nebula_error::ErrorCategory::External,
            Self::InvalidInput(_) => nebula_error::ErrorCategory::Validation,
            Self::RefreshFailed { .. } => nebula_error::ErrorCategory::External,
            Self::RevokeFailed { .. } => nebula_error::ErrorCategory::External,
            Self::CompositionNotAvailable => nebula_error::ErrorCategory::Internal,
            Self::CompositionFailed { .. } => nebula_error::ErrorCategory::External,
            Self::SchemeMismatch { .. } => nebula_error::ErrorCategory::Validation,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::Crypto { source } => nebula_error::Classify::code(source),
            Self::Validation { source } => nebula_error::Classify::code(source),
            Self::NotInteractive => nebula_error::ErrorCode::new("CREDENTIAL:NOT_INTERACTIVE"),
            Self::Provider(_) => nebula_error::ErrorCode::new("CREDENTIAL:PROVIDER"),
            Self::InvalidInput(_) => nebula_error::ErrorCode::new("CREDENTIAL:INVALID_INPUT"),
            Self::RefreshFailed { .. } => nebula_error::ErrorCode::new("CREDENTIAL:REFRESH_FAILED"),
            Self::RevokeFailed { .. } => nebula_error::ErrorCode::new("CREDENTIAL:REVOKE_FAILED"),
            Self::CompositionNotAvailable => {
                nebula_error::ErrorCode::new("CREDENTIAL:COMPOSITION_UNAVAILABLE")
            }
            Self::CompositionFailed { .. } => {
                nebula_error::ErrorCode::new("CREDENTIAL:COMPOSITION_FAILED")
            }
            Self::SchemeMismatch { .. } => {
                nebula_error::ErrorCode::new("CREDENTIAL:SCHEME_MISMATCH")
            }
        }
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::Crypto { .. } | Self::Validation { .. } | Self::NotInteractive => false,
            Self::Provider(_) => false,
            Self::RefreshFailed { kind, .. } => matches!(
                kind,
                RefreshErrorKind::TransientNetwork | RefreshErrorKind::ProviderUnavailable
            ),
            Self::InvalidInput(_)
            | Self::RevokeFailed { .. }
            | Self::CompositionNotAvailable
            | Self::CompositionFailed { .. }
            | Self::SchemeMismatch { .. } => false,
        }
    }
}

// CryptoError, ValidationError: Classify derived via #[derive(nebula_error::Classify)]

/// Result type alias for credential operations
pub type Result<T> = std::result::Result<T, CredentialError>;

// Conversion helpers for ergonomic error propagation
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
    fn refresh_error_convenience() {
        let err = CredentialError::refresh(
            RefreshErrorKind::TokenExpired,
            RetryAdvice::Never,
            "refresh token expired",
        );
        assert!(matches!(
            err,
            CredentialError::RefreshFailed {
                kind: RefreshErrorKind::TokenExpired,
                ..
            }
        ));
        assert!(err.to_string().contains("refresh failed"));
    }

    #[test]
    fn scheme_mismatch_error() {
        let err = CredentialError::SchemeMismatch {
            expected: "bearer",
            actual: "database".to_string(),
        };
        assert!(err.to_string().contains("bearer"));
        assert!(err.to_string().contains("database"));
    }

    #[test]
    fn refresh_transient_is_retryable() {
        use nebula_error::Classify;

        let err = CredentialError::refresh(
            RefreshErrorKind::TransientNetwork,
            RetryAdvice::Immediate,
            "connection reset",
        );
        assert!(err.is_retryable());

        let err = CredentialError::refresh(
            RefreshErrorKind::TokenExpired,
            RetryAdvice::Never,
            "expired",
        );
        assert!(!err.is_retryable());
    }
}
