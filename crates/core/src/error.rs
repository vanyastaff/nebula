//! Error handling for Nebula core
//!
//! This module provides centralized error types and error handling
//! utilities used throughout the Nebula system.
//!
//! # Examples
//!
//! ```
//! use nebula_core::CoreError;
//!
//! let err = CoreError::validation("invalid format");
//! assert_eq!(err.error_code(), "VALIDATION_ERROR");
//! assert!(!err.is_retryable());
//!
//! let not_found = CoreError::not_found("workflow", "abc-123");
//! assert_eq!(not_found.error_code(), "NOT_FOUND_ERROR");
//! ```

use thiserror::Error;

use super::id::{TenantId, UserId};

/// Core error type for Nebula
#[derive(Error, Debug, Clone)]
pub enum CoreError {
    /// Validation error
    #[error("Validation error: {message}")]
    Validation {
        message: String,
        field: Option<String>,
        value: Option<String>,
    },

    /// Not found error
    #[error("Resource not found: {resource_type} '{resource_id}'")]
    NotFound {
        resource_type: String,
        resource_id: String,
    },

    /// Already exists error
    #[error("Resource already exists: {resource_type} '{resource_id}'")]
    AlreadyExists {
        resource_type: String,
        resource_id: String,
    },

    /// Permission denied error
    #[error("Permission denied: {operation} on {resource}")]
    PermissionDenied {
        operation: String,
        resource: String,
        reason: Option<String>,
    },

    /// Authentication error
    #[error("Authentication failed: {reason}")]
    Authentication {
        reason: String,
        user_id: Option<UserId>,
    },

    /// Authorization error
    #[error("Authorization failed: {operation} on {resource}")]
    Authorization {
        operation: String,
        resource: String,
        user_id: Option<UserId>,
        tenant_id: Option<TenantId>,
    },

    /// Invalid input error
    #[error("Invalid input: {message}")]
    InvalidInput {
        message: String,
        field: Option<String>,
        value: Option<String>,
    },

    /// Serialization error
    #[error("Serialization failed: {message}")]
    Serialization {
        message: String,
        format: Option<String>,
    },

    /// Deserialization error
    #[error("Deserialization failed: {message}")]
    Deserialization {
        message: String,
        format: Option<String>,
        data: Option<String>,
    },

    /// Timeout error
    #[error("Operation timed out after {duration:?}: {operation}")]
    Timeout {
        operation: String,
        duration: std::time::Duration,
    },

    /// Resource exhausted error
    #[error("Resource exhausted: {resource} (limit: {limit})")]
    ResourceExhausted {
        resource: String,
        limit: String,
        current: Option<String>,
    },

    /// Internal error
    #[error("Internal error: {message}")]
    Internal {
        message: String,
        code: Option<String>,
    },

    /// Configuration error
    #[error("Configuration error: {message}")]
    Configuration {
        message: String,
        file: Option<String>,
        line: Option<u32>,
    },

    /// State error
    #[error("Invalid state: {current_state} for operation {operation}")]
    InvalidState {
        current_state: String,
        expected_state: Option<String>,
        operation: String,
    },

    /// Dependency error
    #[error("Dependency error: {dependency} - {reason}")]
    Dependency {
        dependency: String,
        reason: String,
        operation: Option<String>,
    },

}

impl CoreError {
    /// Check if this error is retryable.
    ///
    /// Domain-specific retryable errors (rate limiting, network, storage) are handled
    /// by the respective crate error types.
    pub fn is_retryable(&self) -> bool {
        matches!(self, CoreError::Timeout { .. })
    }

    /// Check if this error is a client error (4xx equivalent).
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            CoreError::Validation { .. }
                | CoreError::NotFound { .. }
                | CoreError::AlreadyExists { .. }
                | CoreError::PermissionDenied { .. }
                | CoreError::Authentication { .. }
                | CoreError::Authorization { .. }
                | CoreError::InvalidInput { .. }
                | CoreError::ResourceExhausted { .. }
                | CoreError::InvalidState { .. }
        )
    }

    /// Check if this error is a server error (5xx equivalent).
    pub fn is_server_error(&self) -> bool {
        matches!(
            self,
            CoreError::Internal { .. }
                | CoreError::Configuration { .. }
                | CoreError::Dependency { .. }
        )
    }

    /// Get the error code for this error.
    pub fn error_code(&self) -> &'static str {
        match self {
            CoreError::Validation { .. } => "VALIDATION_ERROR",
            CoreError::NotFound { .. } => "NOT_FOUND_ERROR",
            CoreError::AlreadyExists { .. } => "ALREADY_EXISTS_ERROR",
            CoreError::PermissionDenied { .. } => "PERMISSION_DENIED_ERROR",
            CoreError::Authentication { .. } => "AUTHENTICATION_ERROR",
            CoreError::Authorization { .. } => "AUTHORIZATION_ERROR",
            CoreError::InvalidInput { .. } => "INVALID_INPUT_ERROR",
            CoreError::Serialization { .. } => "SERIALIZATION_ERROR",
            CoreError::Deserialization { .. } => "DESERIALIZATION_ERROR",
            CoreError::Timeout { .. } => "TIMEOUT_ERROR",
            CoreError::ResourceExhausted { .. } => "RESOURCE_EXHAUSTED_ERROR",
            CoreError::Internal { .. } => "INTERNAL_ERROR",
            CoreError::Configuration { .. } => "CONFIGURATION_ERROR",
            CoreError::InvalidState { .. } => "INVALID_STATE_ERROR",
            CoreError::Dependency { .. } => "DEPENDENCY_ERROR",
        }
    }

    /// Get a user-friendly error message
    pub fn user_message(&self) -> String {
        match self {
            CoreError::Validation { message, .. } => format!("Invalid input: {}", message),
            CoreError::NotFound {
                resource_type,
                resource_id,
                ..
            } => {
                format!("{} '{}' not found", resource_type, resource_id)
            }
            CoreError::AlreadyExists {
                resource_type,
                resource_id,
                ..
            } => {
                format!("{} '{}' already exists", resource_type, resource_id)
            }
            CoreError::PermissionDenied {
                operation,
                resource,
                ..
            } => {
                format!("You don't have permission to {} {}", operation, resource)
            }
            CoreError::Authentication { reason, .. } => {
                format!("Authentication failed: {}", reason)
            }
            CoreError::Authorization {
                operation,
                resource,
                ..
            } => {
                format!("You don't have permission to {} {}", operation, resource)
            }
            CoreError::InvalidInput { message, .. } => format!("Invalid input: {}", message),
            CoreError::Serialization { .. } => "Failed to process data".to_string(),
            CoreError::Deserialization { .. } => "Failed to process data".to_string(),
            CoreError::Timeout { operation, .. } => format!("{} timed out", operation),
            CoreError::ResourceExhausted { resource, .. } => format!("{} limit reached", resource),
            CoreError::Internal { .. } => {
                "An internal error occurred. Please try again later.".to_string()
            }
            CoreError::Configuration { message, .. } => format!("Configuration error: {}", message),
            CoreError::InvalidState {
                current_state,
                operation,
                ..
            } => {
                format!("Cannot {} in current state: {}", operation, current_state)
            }
            CoreError::Dependency { dependency, .. } => format!("{} is not available", dependency),
        }
    }

    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        CoreError::Validation {
            message: message.into(),
            field: None,
            value: None,
        }
    }

    /// Create a validation error with field and value
    pub fn validation_with_details(
        message: impl Into<String>,
        field: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        CoreError::Validation {
            message: message.into(),
            field: Some(field.into()),
            value: Some(value.into()),
        }
    }

    /// Create a not found error
    pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        CoreError::NotFound {
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
        }
    }

    /// Create an already exists error
    pub fn already_exists(
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
    ) -> Self {
        CoreError::AlreadyExists {
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
        }
    }

    /// Create a permission denied error
    pub fn permission_denied(
        operation: impl Into<String>,
        resource: impl Into<String>,
        reason: Option<impl Into<String>>,
    ) -> Self {
        CoreError::PermissionDenied {
            operation: operation.into(),
            resource: resource.into(),
            reason: reason.map(|r| r.into()),
        }
    }

    /// Create an authentication error
    pub fn authentication(reason: impl Into<String>, user_id: Option<UserId>) -> Self {
        CoreError::Authentication {
            reason: reason.into(),
            user_id,
        }
    }

    /// Create an authorization error
    pub fn authorization(
        operation: impl Into<String>,
        resource: impl Into<String>,
        user_id: Option<UserId>,
        tenant_id: Option<TenantId>,
    ) -> Self {
        CoreError::Authorization {
            operation: operation.into(),
            resource: resource.into(),
            user_id,
            tenant_id,
        }
    }

    /// Create an invalid input error
    pub fn invalid_input(message: impl Into<String>) -> Self {
        CoreError::InvalidInput {
            message: message.into(),
            field: None,
            value: None,
        }
    }

    /// Create a timeout error
    pub fn timeout(operation: impl Into<String>, duration: std::time::Duration) -> Self {
        CoreError::Timeout {
            operation: operation.into(),
            duration,
        }
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        CoreError::Internal {
            message: message.into(),
            code: None,
        }
    }
}

/// Result type for operations that can fail with a CoreError.
pub type CoreResult<T> = Result<T, CoreError>;

/// Error conversion traits
impl From<std::io::Error> for CoreError {
    fn from(err: std::io::Error) -> Self {
        CoreError::Internal {
            message: format!("I/O error: {}", err),
            code: Some("IO_ERROR".to_string()),
        }
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(err: serde_json::Error) -> Self {
        CoreError::Serialization {
            message: format!("JSON error: {}", err),
            format: Some("json".to_string()),
        }
    }
}

impl From<postcard::Error> for CoreError {
    fn from(err: postcard::Error) -> Self {
        CoreError::Serialization {
            message: format!("Postcard (binary) error: {}", err),
            format: Some("postcard".to_string()),
        }
    }
}

impl From<chrono::ParseError> for CoreError {
    fn from(err: chrono::ParseError) -> Self {
        CoreError::InvalidInput {
            message: format!("Date/time parsing error: {}", err),
            field: None,
            value: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_error_creation() {
        let validation_error = CoreError::validation("Invalid input");
        assert!(validation_error.is_client_error());
        assert!(!validation_error.is_retryable());

        let timeout_error = CoreError::timeout("database query", Duration::from_secs(5));
        assert!(timeout_error.is_retryable());
        assert!(!timeout_error.is_client_error());

        let internal_error = CoreError::internal("Something went wrong");
        assert!(internal_error.is_server_error());
        assert!(!internal_error.is_retryable());
    }

    #[test]
    fn test_error_codes() {
        let validation_error = CoreError::validation("test");
        assert_eq!(validation_error.error_code(), "VALIDATION_ERROR");

        let not_found_error = CoreError::not_found("User", "123");
        assert_eq!(not_found_error.error_code(), "NOT_FOUND_ERROR");

        let internal_error = CoreError::internal("test");
        assert_eq!(internal_error.error_code(), "INTERNAL_ERROR");
    }

    #[test]
    fn test_user_messages() {
        let validation_error = CoreError::validation("Field is required");
        assert_eq!(
            validation_error.user_message(),
            "Invalid input: Field is required"
        );

        let not_found_error = CoreError::not_found("User", "123");
        assert_eq!(not_found_error.user_message(), "User '123' not found");

        let permission_error = CoreError::permission_denied("read", "document", None::<String>);
        assert_eq!(
            permission_error.user_message(),
            "You don't have permission to read document"
        );
    }

    #[test]
    fn test_error_conversion() {
        // I/O errors convert to Internal (server error)
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let core_error: CoreError = io_error.into();
        assert!(core_error.is_server_error());

        // JSON errors convert to Serialization
        let json_error = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let core_error: CoreError = json_error.into();
        assert!(matches!(core_error, CoreError::Serialization { .. }));
    }

    #[test]
    fn test_retryable_errors() {
        let timeout_error = CoreError::timeout("operation", Duration::from_secs(5));
        assert!(timeout_error.is_retryable());

        let validation_error = CoreError::validation("test");
        assert!(!validation_error.is_retryable());

        let internal_error = CoreError::internal("server error");
        assert!(!internal_error.is_retryable());
    }
}
