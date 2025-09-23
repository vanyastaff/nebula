//! Main NebulaError struct and core error functionality

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

use crate::core::context::ErrorContext;
use crate::kinds::ErrorKind;
use crate::core::traits::{ErrorClassification, ErrorCode, RetryableError};

/// Main error type for Nebula
///
/// This is the primary error type used throughout the Nebula ecosystem.
/// It provides structured error information with rich context and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NebulaError {
    /// The specific kind/variant of error
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
    /// Create a new NebulaError with the given kind
    pub fn new(kind: ErrorKind) -> Self {
        let retryable = kind.is_retryable();
        let code = kind.error_code().to_string();
        let message = kind.to_string();

        Self {
            kind,
            context: None,
            retryable,
            retry_after: None,
            code,
            message,
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

    /// Set retry information
    pub fn with_retry_info(mut self, retryable: bool, retry_after: Option<Duration>) -> Self {
        self.retryable = retryable;
        self.retry_after = retry_after;
        self
    }

    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Check if this is a client error (4xx equivalent)
    pub fn is_client_error(&self) -> bool {
        self.kind.is_client_error()
    }

    /// Check if this is a server error (5xx equivalent)
    pub fn is_server_error(&self) -> bool {
        self.kind.is_server_error()
    }

    /// Check if this is a system error (infrastructure/system level)
    pub fn is_system_error(&self) -> bool {
        self.kind.is_system_error()
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

    /// Get the error details
    pub fn details(&self) -> Option<&str> {
        self.details.as_deref()
    }

    /// Get the error context
    pub fn context(&self) -> Option<&ErrorContext> {
        self.context.as_ref()
    }

    // =============================================================================
    // Convenience Constructor Methods
    // =============================================================================

    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(crate::kinds::ClientError::validation(message)))
    }

    /// Create a not found error
    pub fn not_found(resource_type: impl Into<String>, resource_id: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(crate::kinds::ClientError::not_found(resource_type, resource_id)))
    }

    /// Create a permission denied error
    pub fn permission_denied(operation: impl Into<String>, resource: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(crate::kinds::ClientError::permission_denied(operation, resource)))
    }

    /// Create an authentication error
    pub fn authentication(reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Client(crate::kinds::ClientError::authentication(reason)))
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Server(crate::kinds::ServerError::internal(message)))
    }

    /// Create a service unavailable error
    pub fn service_unavailable(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new(ErrorKind::Server(crate::kinds::ServerError::service_unavailable(service, reason)))
    }

    /// Create a timeout error
    pub fn timeout(operation: impl Into<String>, duration: std::time::Duration) -> Self {
        Self::new(ErrorKind::System(crate::kinds::SystemError::timeout(operation, duration)))
    }

    /// Create a rate limit exceeded error
    pub fn rate_limit_exceeded(limit: u32, period: std::time::Duration) -> Self {
        Self::new(ErrorKind::System(crate::kinds::SystemError::rate_limit_exceeded(limit, period)))
    }

    /// Create a network error
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::System(crate::kinds::SystemError::network(message)))
    }

    /// Create a database error
    pub fn database(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::System(crate::kinds::SystemError::database(message)))
    }
}

impl std::error::Error for NebulaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
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
            write!(f, " [Retryable")?;
            if let Some(retry_after) = self.retry_after {
                write!(f, " after {:?}", retry_after)?;
            }
            write!(f, "]")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let error = NebulaError::validation("Invalid input");

        assert_eq!(error.error_code(), "VALIDATION_ERROR");
        assert!(!error.is_retryable());
        assert!(error.is_client_error());
        assert!(!error.is_server_error());
    }

    #[test]
    fn test_error_with_context() {
        let context = ErrorContext::new("Processing user request");
        let error = NebulaError::internal("Database error").with_context(context);

        assert!(error.context.is_some());
        assert_eq!(error.context().unwrap().description, "Processing user request");
    }

    #[test]
    fn test_error_display() {
        let error = NebulaError::timeout("API call", Duration::from_secs(30))
            .with_details("Connection to external service timed out");

        let display = format!("{}", error);
        assert!(display.contains("TIMEOUT_ERROR"));
        assert!(display.contains("API call"));
        assert!(display.contains("Connection to external service timed out"));
        assert!(display.contains("[Retryable"));
    }
}