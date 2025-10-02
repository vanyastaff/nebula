//! Core error types for the validation system

use std::fmt;
use thiserror::Error;

/// Core errors that can occur in the validation system
#[derive(Debug, Error)]
pub enum CoreError {
    /// Validation expired
    #[error("Validation proof has expired")]
    ValidationExpired,

    /// Invalid proof
    #[error("Invalid validation proof: {0}")]
    InvalidProof(String),

    /// Recovery failed
    #[error("Failed to recover from invalid state: {0}")]
    RecoveryFailed(String),

    /// Signature verification failed
    #[error("Signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    /// Context error
    #[error("Context error: {0}")]
    ContextError(String),

    /// Type conversion error
    #[error("Type conversion failed: {0}")]
    TypeConversion(String),

    /// Conversion error
    #[error("Conversion error: {0}")]
    ConversionError(String),

    /// Proof building error
    #[error("Failed to build proof: {0}")]
    ProofBuildError(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Other errors
    #[error("Core error: {0}")]
    Other(String),
}

/// Enhanced validation error for core validators
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Error message
    pub message: String,
    /// Error code (optional)
    pub code: Option<String>,
    /// Field path (optional)
    pub path: Option<String>,
    /// Validator name that generated this error (optional)
    pub validator: Option<String>,
    /// Additional error details (optional)
    pub details: Option<serde_json::Value>,
}

impl ValidationError {
    /// Create a new validation error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            path: None,
            validator: None,
            details: None,
        }
    }

    /// Set error code
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Set field path
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Set validator name
    pub fn with_validator(mut self, validator: impl Into<String>) -> Self {
        self.validator = Some(validator.into());
        self
    }

    /// Set additional details
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ValidationError {}

/// Simple validator ID
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidatorId(String);

impl ValidatorId {
    /// Create a new validator ID
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the ID as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ValidatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ValidatorId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ValidatorId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Result type for core operations
pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    /// Create a validation expired error
    pub fn expired() -> Self {
        CoreError::ValidationExpired
    }

    /// Create an invalid proof error
    pub fn invalid_proof(message: impl Into<String>) -> Self {
        CoreError::InvalidProof(message.into())
    }

    /// Create a recovery failed error
    pub fn recovery_failed(message: impl Into<String>) -> Self {
        CoreError::RecoveryFailed(message.into())
    }

    /// Create a context error
    pub fn context(message: impl Into<String>) -> Self {
        CoreError::ContextError(message.into())
    }

    /// Create a conversion error
    pub fn conversion(message: impl Into<String>) -> Self {
        CoreError::ConversionError(message.into())
    }

    /// Check if this is an expiration error
    pub fn is_expired(&self) -> bool {
        matches!(self, CoreError::ValidationExpired)
    }

    /// Check if this is a recovery error
    pub fn is_recovery_failed(&self) -> bool {
        matches!(self, CoreError::RecoveryFailed(_))
    }
}

// Conversion from CoreError to ValidationError
impl From<CoreError> for ValidationError {
    fn from(error: CoreError) -> Self {
        let message = match error {
            CoreError::ValidationExpired => "Validation has expired".to_string(),
            CoreError::InvalidProof(msg) => format!("Invalid proof: {}", msg),
            CoreError::RecoveryFailed(msg) => format!("Recovery failed: {}", msg),
            CoreError::SignatureVerificationFailed(msg) => {
                format!("Signature verification failed: {}", msg)
            }
            CoreError::ContextError(msg) => format!("Context error: {}", msg),
            CoreError::TypeConversion(msg) => format!("Type conversion failed: {}", msg),
            CoreError::ConversionError(msg) => format!("Conversion error: {}", msg),
            CoreError::ProofBuildError(msg) => format!("Failed to build proof: {}", msg),
            CoreError::SerializationError(err) => format!("Serialization error: {}", err),
            CoreError::Other(msg) => format!("Core error: {}", msg),
        };

        ValidationError::new(message)
    }
}
