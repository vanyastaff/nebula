//! Client error types (4xx equivalent)
//!
//! These errors are typically caused by invalid user input, authentication failures,
//! or other client-side issues. They are generally not retryable.

#![allow(missing_docs)] // Enum variant fields are self-explanatory

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::core::traits::{ErrorCode, RetryableError};
use crate::kinds::codes;

/// Client-side error variants
#[non_exhaustive]
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ClientError {
    /// Validation error
    #[error("Validation error: {message}")]
    Validation { message: String },

    /// Resource not found
    #[error("Resource not found: {resource_type} '{resource_id}'")]
    NotFound {
        resource_type: String,
        resource_id: String,
    },

    /// Invalid input
    #[error("Invalid input: {message}")]
    InvalidInput { message: String },

    /// Permission denied
    #[error("Permission denied: {operation} on {resource}")]
    PermissionDenied { operation: String, resource: String },

    /// Authentication failed
    #[error("Authentication failed: {reason}")]
    Authentication { reason: String },

    /// Authorization failed
    #[error("Authorization failed: {operation} on {resource}")]
    Authorization { operation: String, resource: String },

    /// Serialization error
    #[error("Serialization failed: {message}")]
    Serialization { message: String },

    /// Deserialization error
    #[error("Deserialization failed: {message}")]
    Deserialization { message: String },

    /// Conflict error (resource already exists, etc.)
    #[error("Conflict: {message}")]
    Conflict { message: String },

    /// Precondition failed
    #[error("Precondition failed: {condition}")]
    PreconditionFailed { condition: String },

    /// Request too large
    #[error("Request entity too large: {size} bytes exceeds limit of {limit} bytes")]
    RequestTooLarge { size: u64, limit: u64 },

    /// Unsupported media type
    #[error("Unsupported media type: {media_type}")]
    UnsupportedMediaType { media_type: String },
}

impl RetryableError for ClientError {
    fn is_retryable(&self) -> bool {
        // Most client errors are not retryable as they indicate user error
        // The exception might be some authentication errors due to token expiry
        match self {
            ClientError::Authentication { .. } => true, // Token might have expired
            _ => false,
        }
    }

    fn retry_delay(&self) -> Option<Duration> {
        match self {
            ClientError::Authentication { .. } => Some(Duration::from_secs(1)),
            _ => None,
        }
    }
}

impl ErrorCode for ClientError {
    fn error_code(&self) -> &str {
        match self {
            ClientError::Validation { .. } => codes::VALIDATION_ERROR,
            ClientError::NotFound { .. } => codes::NOT_FOUND_ERROR,
            ClientError::InvalidInput { .. } => codes::INVALID_INPUT_ERROR,
            ClientError::PermissionDenied { .. } => codes::PERMISSION_DENIED_ERROR,
            ClientError::Authentication { .. } => codes::AUTHENTICATION_ERROR,
            ClientError::Authorization { .. } => codes::AUTHORIZATION_ERROR,
            ClientError::Serialization { .. } => codes::SERIALIZATION_ERROR,
            ClientError::Deserialization { .. } => codes::DESERIALIZATION_ERROR,
            ClientError::Conflict { .. } => codes::CONFLICT_ERROR,
            ClientError::PreconditionFailed { .. } => codes::PRECONDITION_FAILED_ERROR,
            ClientError::RequestTooLarge { .. } => codes::REQUEST_TOO_LARGE_ERROR,
            ClientError::UnsupportedMediaType { .. } => codes::UNSUPPORTED_MEDIA_TYPE_ERROR,
        }
    }

    fn error_category(&self) -> &'static str {
        codes::CATEGORY_CLIENT
    }
}

impl ClientError {
    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    /// Create a not found error
    pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        Self::NotFound {
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
        }
    }

    /// Create an invalid input error
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput {
            message: message.into(),
        }
    }

    /// Create a permission denied error
    pub fn permission_denied(operation: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::PermissionDenied {
            operation: operation.into(),
            resource: resource.into(),
        }
    }

    /// Create an authentication error
    pub fn authentication(reason: impl Into<String>) -> Self {
        Self::Authentication {
            reason: reason.into(),
        }
    }

    /// Create an authorization error
    pub fn authorization(operation: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::Authorization {
            operation: operation.into(),
            resource: resource.into(),
        }
    }

    /// Create a serialization error
    pub fn serialization(message: impl Into<String>) -> Self {
        Self::Serialization {
            message: message.into(),
        }
    }

    /// Create a deserialization error
    pub fn deserialization(message: impl Into<String>) -> Self {
        Self::Deserialization {
            message: message.into(),
        }
    }

    /// Create a conflict error
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    /// Create a precondition failed error
    pub fn precondition_failed(condition: impl Into<String>) -> Self {
        Self::PreconditionFailed {
            condition: condition.into(),
        }
    }

    /// Create a request too large error
    pub fn request_too_large(size: u64, limit: u64) -> Self {
        Self::RequestTooLarge { size, limit }
    }

    /// Create an unsupported media type error
    pub fn unsupported_media_type(media_type: impl Into<String>) -> Self {
        Self::UnsupportedMediaType {
            media_type: media_type.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_error_creation() {
        let validation_error = ClientError::validation("Missing required field 'name'");
        assert_eq!(validation_error.error_code(), "VALIDATION_ERROR");
        assert!(!validation_error.is_retryable());

        let not_found_error = ClientError::not_found("User", "123");
        assert_eq!(not_found_error.error_code(), "NOT_FOUND_ERROR");
        assert!(!not_found_error.is_retryable());

        let auth_error = ClientError::authentication("Token expired");
        assert_eq!(auth_error.error_code(), "AUTHENTICATION_ERROR");
        assert!(auth_error.is_retryable()); // Authentication errors might be retryable
    }

    #[test]
    fn test_client_error_display() {
        let validation_error = ClientError::validation("Invalid email format");
        assert_eq!(
            validation_error.to_string(),
            "Validation error: Invalid email format"
        );

        let not_found_error = ClientError::not_found("User", "user123");
        assert_eq!(
            not_found_error.to_string(),
            "Resource not found: User 'user123'"
        );

        let permission_error = ClientError::permission_denied("read", "sensitive_data");
        assert_eq!(
            permission_error.to_string(),
            "Permission denied: read on sensitive_data"
        );
    }

    #[test]
    fn test_retry_behavior() {
        let validation_error = ClientError::validation("Invalid input");
        assert!(!validation_error.is_retryable());
        assert_eq!(validation_error.retry_delay(), None);

        let auth_error = ClientError::authentication("Token expired");
        assert!(auth_error.is_retryable());
        assert_eq!(auth_error.retry_delay(), Some(Duration::from_secs(1)));
    }
}
