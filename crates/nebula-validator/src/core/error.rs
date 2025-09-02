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
impl From<CoreError> for crate::types::ValidationError {
    fn from(error: CoreError) -> Self {
        use crate::types::{ValidationError, ErrorCode};
        
        match error {
            CoreError::ValidationExpired => {
                ValidationError::new(ErrorCode::Expired, "Validation has expired")
            },
            CoreError::InvalidProof(msg) => {
                ValidationError::new(ErrorCode::InvalidProof, msg)
            },
            CoreError::RecoveryFailed(msg) => {
                ValidationError::new(ErrorCode::RecoveryFailed, msg)
            },
            CoreError::SignatureVerificationFailed(msg) => {
                ValidationError::new(ErrorCode::SignatureFailed, msg)
            },
            CoreError::ContextError(msg) => {
                ValidationError::new(ErrorCode::ContextError, msg)
            },
            CoreError::ConversionError(msg) => {
                ValidationError::new(ErrorCode::ConversionError, msg)
            },
            CoreError::ProofBuildError(msg) => {
                ValidationError::new(ErrorCode::ProofError, msg)
            },
            CoreError::SerializationError(err) => {
                ValidationError::new(ErrorCode::SerializationError, err.to_string())
            },
            CoreError::Other(msg) => {
                ValidationError::new(ErrorCode::InternalError, msg)
            },
        }
    }
}