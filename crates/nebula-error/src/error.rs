//! Core error types for Nebula

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;
use thiserror::Error;

use super::context::ErrorContext;

/// Main error type for Nebula
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub struct NebulaError {
    /// The kind of error
    pub kind: ErrorKind,
    /// Additional context information
    pub context: Option<ErrorContext>,
    /// Whether this error is retryable
    pub retryable: bool,
    /// Suggested retry delay
    pub retry_after: Option<Duration>,
    /// Error code for programmatic handling
    pub code: String,
    /// User-friendly error message
    pub message: String,
    /// Technical details for debugging
    pub details: Option<String>,
}

impl NebulaError {
    /// Create a new validation error
    pub fn validation(message: impl Into<String>) -> Self {
        let message_str = message.into();
        Self {
            kind: ErrorKind::Validation {
                message: message_str.clone(),
            },
            context: None,
            retryable: false,
            retry_after: None,
            code: "VALIDATION_ERROR".to_string(),
            message: message_str,
            details: None,
        }
    }

    /// Create a new not found error
    pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        let resource_type = resource_type.into();
        let resource_id = resource_id.into();
        Self {
            kind: ErrorKind::NotFound {
                resource_type: resource_type.clone(),
                resource_id: resource_id.clone(),
            },
            context: None,
            retryable: false,
            retry_after: None,
            code: "NOT_FOUND_ERROR".to_string(),
            message: format!("{} '{}' not found", resource_type, resource_id),
            details: None,
        }
    }

    /// Create a new timeout error
    pub fn timeout(operation: impl Into<String>, duration: Duration) -> Self {
        let operation = operation.into();
        Self {
            kind: ErrorKind::Timeout {
                operation: operation.clone(),
                duration,
            },
            context: None,
            retryable: true,
            retry_after: Some(duration),
            code: "TIMEOUT_ERROR".to_string(),
            message: format!("Operation '{}' timed out after {:?}", operation, duration),
            details: None,
        }
    }

    /// Create a new rate limit error
    pub fn rate_limit_exceeded(
        limit: u32,
        period: Duration,
        retry_after: Option<Duration>,
    ) -> Self {
        Self {
            kind: ErrorKind::RateLimitExceeded { limit, period },
            context: None,
            retryable: true,
            retry_after,
            code: "RATE_LIMIT_ERROR".to_string(),
            message: format!("Rate limit exceeded: {} requests per {:?}", limit, period),
            details: None,
        }
    }

    /// Create a new internal error
    pub fn internal(message: impl Into<String>) -> Self {
        let message_str = message.into();
        Self {
            kind: ErrorKind::Internal {
                message: message_str.clone(),
            },
            context: None,
            retryable: false,
            retry_after: None,
            code: "INTERNAL_ERROR".to_string(),
            message: message_str,
            details: None,
        }
    }

    /// Create a new service unavailable error
    pub fn service_unavailable(
        service: impl Into<String>,
        reason: impl Into<String>,
        retry_after: Option<Duration>,
    ) -> Self {
        let service = service.into();
        let reason = reason.into();
        Self {
            kind: ErrorKind::ServiceUnavailable {
                service: service.clone(),
                reason: reason.clone(),
            },
            context: None,
            retryable: true,
            retry_after,
            code: "SERVICE_UNAVAILABLE_ERROR".to_string(),
            message: format!("Service '{}' unavailable: {}", service, reason),
            details: None,
        }
    }

    /// Create a new permission denied error
    pub fn permission_denied(operation: impl Into<String>, resource: impl Into<String>) -> Self {
        let operation = operation.into();
        let resource = resource.into();
        Self {
            kind: ErrorKind::PermissionDenied {
                operation: operation.clone(),
                resource: resource.clone(),
            },
            context: None,
            retryable: false,
            retry_after: None,
            code: "PERMISSION_DENIED_ERROR".to_string(),
            message: format!("Permission denied: {} on {}", operation, resource),
            details: None,
        }
    }

    /// Create a new network error
    pub fn network(message: impl Into<String>) -> Self {
        let message_str = message.into();
        Self {
            kind: ErrorKind::Network {
                message: message_str.clone(),
            },
            context: None,
            retryable: true,
            retry_after: None,
            code: "NETWORK_ERROR".to_string(),
            message: message_str,
            details: None,
        }
    }

    /// Add context to the error
    pub fn with_context(mut self, context: ErrorContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Add details to the error
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Check if this is a client error (4xx)
    pub fn is_client_error(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Validation { .. }
                | ErrorKind::NotFound { .. }
                | ErrorKind::InvalidInput { .. }
                | ErrorKind::PermissionDenied { .. }
        )
    }

    /// Check if this is a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Internal { .. }
                | ErrorKind::ServiceUnavailable { .. }
                | ErrorKind::Timeout { .. }
        )
    }

    /// Get the suggested retry delay
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// Get the error code
    pub fn error_code(&self) -> &str {
        &self.code
    }

    /// Get the user-friendly message
    pub fn user_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for NebulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;

        if let Some(ref context) = self.context {
            write!(f, " (Context: {})", context)?;
        }

        if let Some(ref details) = self.details {
            write!(f, " - {}", details)?;
        }

        if self.retryable {
            write!(f, " [Retryable]")?;
        }

        Ok(())
    }
}

/// Specific error kinds
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ErrorKind {
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

    /// Timeout error
    #[error("Operation timed out: {operation} after {duration:?}")]
    Timeout {
        operation: String,
        duration: Duration,
    },

    /// Rate limit exceeded
    #[error("Rate limit exceeded: {limit} requests per {period:?}")]
    RateLimitExceeded { limit: u32, period: Duration },

    /// Resource exhausted
    #[error("Resource exhausted: {resource}")]
    ResourceExhausted { resource: String },

    /// Service unavailable
    #[error("Service unavailable: {service} - {reason}")]
    ServiceUnavailable { service: String, reason: String },

    /// Internal error
    #[error("Internal error: {message}")]
    Internal { message: String },

    /// Network error
    #[error("Network error: {message}")]
    Network { message: String },

    /// Database error
    #[error("Database error: {message}")]
    Database { message: String },

    /// External service error
    #[error("External service error: {service} - {message}")]
    ExternalService { service: String, message: String },
}

/// Result type for Nebula operations
pub type Result<T> = std::result::Result<T, NebulaError>;

/// Extension trait for adding context to Results
pub trait ResultExt<T, E> {
    /// Add context to a Result
    fn context(self, context: impl Into<String>) -> Result<T>;
}

impl<T, E> ResultExt<T, E> for std::result::Result<T, E>
where
    E: Into<NebulaError>,
{
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|e| {
            let mut nebula_error = e.into();
            nebula_error = nebula_error.with_context(ErrorContext::new(context));
            nebula_error
        })
    }
}

// Implement From for common error types
impl From<std::io::Error> for NebulaError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => NebulaError::not_found("File", "unknown"),
            std::io::ErrorKind::PermissionDenied => NebulaError::permission_denied("read", "file"),
            std::io::ErrorKind::TimedOut => {
                NebulaError::timeout("I/O operation", Duration::from_secs(30))
            }
            _ => NebulaError::internal(format!("I/O error: {}", err)),
        }
    }
}

impl From<serde_json::Error> for NebulaError {
    fn from(err: serde_json::Error) -> Self {
        NebulaError::deserialization(format!("JSON error: {}", err))
    }
}

impl From<bincode::Error> for NebulaError {
    fn from(err: bincode::Error) -> Self {
        NebulaError::deserialization(format!("Bincode error: {}", err))
    }
}

impl From<uuid::Error> for NebulaError {
    fn from(err: uuid::Error) -> Self {
        NebulaError::validation(format!("UUID error: {}", err))
    }
}

impl From<chrono::ParseError> for NebulaError {
    fn from(err: chrono::ParseError) -> Self {
        NebulaError::validation(format!("Date/time parsing error: {}", err))
    }
}

impl From<anyhow::Error> for NebulaError {
    fn from(err: anyhow::Error) -> Self {
        NebulaError::internal(format!("Anyhow error: {}", err))
    }
}

impl From<&str> for NebulaError {
    fn from(err: &str) -> Self {
        NebulaError::internal(err.to_string())
    }
}

impl From<String> for NebulaError {
    fn from(err: String) -> Self {
        NebulaError::internal(err)
    }
}

// Helper methods for ErrorKind
impl ErrorKind {
    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> NebulaError {
        NebulaError::validation(message)
    }

    /// Create a not found error
    pub fn not_found(
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
    ) -> NebulaError {
        NebulaError::not_found(resource_type, resource_id)
    }

    /// Create a timeout error
    pub fn timeout(operation: impl Into<String>, duration: Duration) -> NebulaError {
        NebulaError::timeout(operation, duration)
    }

    /// Create a rate limit error
    pub fn rate_limit_exceeded(
        limit: u32,
        period: Duration,
        retry_after: Option<Duration>,
    ) -> NebulaError {
        NebulaError::rate_limit_exceeded(limit, period, retry_after)
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> NebulaError {
        NebulaError::internal(message)
    }

    /// Create a service unavailable error
    pub fn service_unavailable(
        service: impl Into<String>,
        reason: impl Into<String>,
        retry_after: Option<Duration>,
    ) -> NebulaError {
        NebulaError::service_unavailable(service, reason, retry_after)
    }
}

// Helper methods for NebulaError
impl NebulaError {
    /// Create a deserialization error
    pub fn deserialization(message: impl Into<String>) -> Self {
        let message_str = message.into();
        Self {
            kind: ErrorKind::Deserialization {
                message: message_str.clone(),
            },
            context: None,
            retryable: false,
            retry_after: None,
            code: "DESERIALIZATION_ERROR".to_string(),
            message: message_str,
            details: None,
        }
    }
}
