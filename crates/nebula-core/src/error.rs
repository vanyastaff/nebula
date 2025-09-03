//! Error handling for Nebula core
//!
//! This module provides centralized error types and error handling
//! utilities used throughout the Nebula system.

use std::fmt;
use thiserror::Error;

use super::id::{ExecutionId, NodeId, TenantId, UserId, WorkflowId};

/// Core error type for Nebula
#[derive(Error, Debug, Clone)]
pub enum CoreError {
    /// Validation error
    #[error("Validation error: {message}")]
    Validation { message: String, field: Option<String>, value: Option<String> },

    /// Not found error
    #[error("Resource not found: {resource_type} '{resource_id}'")]
    NotFound { resource_type: String, resource_id: String },

    /// Already exists error
    #[error("Resource already exists: {resource_type} '{resource_id}'")]
    AlreadyExists { resource_type: String, resource_id: String },

    /// Permission denied error
    #[error("Permission denied: {operation} on {resource}")]
    PermissionDenied { operation: String, resource: String, reason: Option<String> },

    /// Authentication error
    #[error("Authentication failed: {reason}")]
    Authentication { reason: String, user_id: Option<UserId> },

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
    InvalidInput { message: String, field: Option<String>, value: Option<String> },

    /// Serialization error
    #[error("Serialization failed: {message}")]
    Serialization { message: String, format: Option<String> },

    /// Deserialization error
    #[error("Deserialization failed: {message}")]
    Deserialization { message: String, format: Option<String>, data: Option<String> },

    /// Timeout error
    #[error("Operation timed out after {duration:?}: {operation}")]
    Timeout { operation: String, duration: std::time::Duration },

    /// Rate limit exceeded error
    #[error("Rate limit exceeded: {limit} requests per {period:?}")]
    RateLimitExceeded {
        limit: u32,
        period: std::time::Duration,
        retry_after: Option<std::time::Duration>,
    },

    /// Resource exhausted error
    #[error("Resource exhausted: {resource} (limit: {limit})")]
    ResourceExhausted { resource: String, limit: String, current: Option<String> },

    /// Internal error
    #[error("Internal error: {message}")]
    Internal { message: String, code: Option<String> },

    /// Service unavailable error
    #[error("Service unavailable: {service} - {reason}")]
    ServiceUnavailable { service: String, reason: String, retry_after: Option<std::time::Duration> },

    /// Configuration error
    #[error("Configuration error: {message}")]
    Configuration { message: String, file: Option<String>, line: Option<u32> },

    /// State error
    #[error("Invalid state: {current_state} for operation {operation}")]
    InvalidState { current_state: String, expected_state: Option<String>, operation: String },

    /// Dependency error
    #[error("Dependency error: {dependency} - {reason}")]
    Dependency { dependency: String, reason: String, operation: Option<String> },

    /// Network error
    #[error("Network error: {operation} - {reason}")]
    Network { operation: String, reason: String, retryable: bool },

    /// Storage error
    #[error("Storage error: {operation} - {reason}")]
    Storage { operation: String, reason: String, backend: Option<String> },

    /// Workflow execution error
    #[error("Workflow execution error: {workflow_id} - {reason}")]
    WorkflowExecution {
        workflow_id: WorkflowId,
        execution_id: Option<ExecutionId>,
        node_id: Option<NodeId>,
        reason: String,
    },

    /// Node execution error
    #[error("Node execution error: {node_id} - {reason}")]
    NodeExecution {
        node_id: NodeId,
        execution_id: Option<ExecutionId>,
        reason: String,
        retryable: bool,
    },

    /// Expression evaluation error
    #[error("Expression evaluation error: {expression} - {reason}")]
    ExpressionEvaluation { expression: String, reason: String, context: Option<String> },

    /// Resource management error
    #[error("Resource management error: {operation} - {reason}")]
    ResourceManagement { operation: String, resource_type: String, reason: String },

    /// Cluster error
    #[error("Cluster error: {operation} - {reason}")]
    Cluster { operation: String, reason: String, node_id: Option<String> },

    /// Tenant error
    #[error("Tenant error: {tenant_id} - {reason}")]
    Tenant { tenant_id: TenantId, reason: String, operation: Option<String> },
}

impl CoreError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            CoreError::Timeout { .. }
                | CoreError::RateLimitExceeded { .. }
                | CoreError::ServiceUnavailable { .. }
                | CoreError::Network { retryable: true, .. }
                | CoreError::Storage { .. }
                | CoreError::NodeExecution { retryable: true, .. }
        )
    }

    /// Check if this error is a client error (4xx)
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
                | CoreError::RateLimitExceeded { .. }
                | CoreError::ResourceExhausted { .. }
                | CoreError::InvalidState { .. }
        )
    }

    /// Check if this error is a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        matches!(
            self,
            CoreError::Internal { .. }
                | CoreError::ServiceUnavailable { .. }
                | CoreError::Configuration { .. }
                | CoreError::Dependency { .. }
                | CoreError::Network { .. }
                | CoreError::Storage { .. }
                | CoreError::Cluster { .. }
        )
    }

    /// Get the error code for this error
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
            CoreError::RateLimitExceeded { .. } => "RATE_LIMIT_ERROR",
            CoreError::ResourceExhausted { .. } => "RESOURCE_EXHAUSTED_ERROR",
            CoreError::Internal { .. } => "INTERNAL_ERROR",
            CoreError::ServiceUnavailable { .. } => "SERVICE_UNAVAILABLE_ERROR",
            CoreError::Configuration { .. } => "CONFIGURATION_ERROR",
            CoreError::InvalidState { .. } => "INVALID_STATE_ERROR",
            CoreError::Dependency { .. } => "DEPENDENCY_ERROR",
            CoreError::Network { .. } => "NETWORK_ERROR",
            CoreError::Storage { .. } => "STORAGE_ERROR",
            CoreError::WorkflowExecution { .. } => "WORKFLOW_EXECUTION_ERROR",
            CoreError::NodeExecution { .. } => "NODE_EXECUTION_ERROR",
            CoreError::ExpressionEvaluation { .. } => "EXPRESSION_EVALUATION_ERROR",
            CoreError::ResourceManagement { .. } => "RESOURCE_MANAGEMENT_ERROR",
            CoreError::Cluster { .. } => "CLUSTER_ERROR",
            CoreError::Tenant { .. } => "TENANT_ERROR",
        }
    }

    /// Get a user-friendly error message
    pub fn user_message(&self) -> String {
        match self {
            CoreError::Validation { message, .. } => format!("Invalid input: {}", message),
            CoreError::NotFound { resource_type, resource_id, .. } => {
                format!("{} '{}' not found", resource_type, resource_id)
            },
            CoreError::AlreadyExists { resource_type, resource_id, .. } => {
                format!("{} '{}' already exists", resource_type, resource_id)
            },
            CoreError::PermissionDenied { operation, resource, .. } => {
                format!("You don't have permission to {} {}", operation, resource)
            },
            CoreError::Authentication { reason, .. } => {
                format!("Authentication failed: {}", reason)
            },
            CoreError::Authorization { operation, resource, .. } => {
                format!("You don't have permission to {} {}", operation, resource)
            },
            CoreError::InvalidInput { message, .. } => format!("Invalid input: {}", message),
            CoreError::Serialization { .. } => "Failed to process data".to_string(),
            CoreError::Deserialization { .. } => "Failed to process data".to_string(),
            CoreError::Timeout { operation, .. } => format!("{} timed out", operation),
            CoreError::RateLimitExceeded { .. } => {
                "Too many requests. Please try again later.".to_string()
            },
            CoreError::ResourceExhausted { resource, .. } => format!("{} limit reached", resource),
            CoreError::Internal { .. } => {
                "An internal error occurred. Please try again later.".to_string()
            },
            CoreError::ServiceUnavailable { service, .. } => {
                format!("{} is temporarily unavailable", service)
            },
            CoreError::Configuration { message, .. } => format!("Configuration error: {}", message),
            CoreError::InvalidState { current_state, operation, .. } => {
                format!("Cannot {} in current state: {}", operation, current_state)
            },
            CoreError::Dependency { dependency, .. } => format!("{} is not available", dependency),
            CoreError::Network { operation, .. } => format!("Network error during {}", operation),
            CoreError::Storage { operation, .. } => format!("Storage error during {}", operation),
            CoreError::WorkflowExecution { workflow_id, reason, .. } => {
                format!("Workflow '{}' execution failed: {}", workflow_id, reason)
            },
            CoreError::NodeExecution { node_id, reason, .. } => {
                format!("Node '{}' execution failed: {}", node_id, reason)
            },
            CoreError::ExpressionEvaluation { .. } => "Expression evaluation failed".to_string(),
            CoreError::ResourceManagement { operation, .. } => {
                format!("Resource operation failed: {}", operation)
            },
            CoreError::Cluster { operation, .. } => {
                format!("Cluster operation failed: {}", operation)
            },
            CoreError::Tenant { tenant_id, reason, .. } => {
                format!("Tenant '{}' operation failed: {}", tenant_id, reason)
            },
        }
    }

    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        CoreError::Validation { message: message.into(), field: None, value: None }
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
        CoreError::NotFound { resource_type: resource_type.into(), resource_id: resource_id.into() }
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
        CoreError::Authentication { reason: reason.into(), user_id }
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
        CoreError::InvalidInput { message: message.into(), field: None, value: None }
    }

    /// Create a timeout error
    pub fn timeout(operation: impl Into<String>, duration: std::time::Duration) -> Self {
        CoreError::Timeout { operation: operation.into(), duration }
    }

    /// Create a rate limit exceeded error
    pub fn rate_limit_exceeded(
        limit: u32,
        period: std::time::Duration,
        retry_after: Option<std::time::Duration>,
    ) -> Self {
        CoreError::RateLimitExceeded { limit, period, retry_after }
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        CoreError::Internal { message: message.into(), code: None }
    }

    /// Create a service unavailable error
    pub fn service_unavailable(
        service: impl Into<String>,
        reason: impl Into<String>,
        retry_after: Option<std::time::Duration>,
    ) -> Self {
        CoreError::ServiceUnavailable {
            service: service.into(),
            reason: reason.into(),
            retry_after,
        }
    }
}

// Display implementation is provided by thiserror

/// Result type for operations that can fail with a CoreError
pub type CoreResult<T> = Result<T, CoreError>;

/// Extension trait for adding context to errors
pub trait ErrorContext<T> {
    /// Add context to an error
    fn with_context<C>(self, _context: C) -> CoreResult<T>
    where
        C: fmt::Display + Send + Sync + 'static;
}

impl<T> ErrorContext<T> for CoreResult<T> {
    fn with_context<C>(self, _context: C) -> CoreResult<T>
    where
        C: fmt::Display + Send + Sync + 'static,
    {
        self.map_err(|e| {
            // For now, we'll just return the original error
            // In the future, we could enhance this to include context
            e
        })
    }
}

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

impl From<bincode::Error> for CoreError {
    fn from(err: bincode::Error) -> Self {
        CoreError::Serialization {
            message: format!("Bincode error: {}", err),
            format: Some("bincode".to_string()),
        }
    }
}

impl From<uuid::Error> for CoreError {
    fn from(err: uuid::Error) -> Self {
        CoreError::InvalidInput {
            message: format!("UUID error: {}", err),
            field: None,
            value: None,
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
        assert_eq!(validation_error.user_message(), "Invalid input: Field is required");

        let not_found_error = CoreError::not_found("User", "123");
        assert_eq!(not_found_error.user_message(), "User '123' not found");

        let permission_error = CoreError::permission_denied("read", "document", None::<String>);
        assert_eq!(permission_error.user_message(), "You don't have permission to read document");
    }

    #[test]
    fn test_error_conversion() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let core_error: CoreError = io_error.into();
        assert!(core_error.is_server_error());

        let json_error = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let core_error: CoreError = json_error.into();
        assert!(core_error.is_server_error());
    }

    #[test]
    fn test_retryable_errors() {
        let timeout_error = CoreError::timeout("operation", Duration::from_secs(5));
        assert!(timeout_error.is_retryable());

        let rate_limit_error = CoreError::rate_limit_exceeded(100, Duration::from_secs(60), None);
        assert!(rate_limit_error.is_retryable());

        let service_unavailable_error =
            CoreError::service_unavailable("database", "maintenance", None);
        assert!(service_unavailable_error.is_retryable());

        let validation_error = CoreError::validation("test");
        assert!(!validation_error.is_retryable());
    }
}
