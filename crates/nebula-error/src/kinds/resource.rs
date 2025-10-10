//! Resource management error types
//!
//! This module provides comprehensive error types for resource management operations
//! including availability, health checks, pooling, dependencies, and lifecycle management.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::core::traits::ErrorCode;
use crate::kinds::codes;

/// Resource management errors
///
/// Covers all resource-related error scenarios including availability, health checks,
/// circuit breakers, pooling, dependencies, and state management.
#[non_exhaustive]
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ResourceError {
    /// Resource is currently unavailable
    #[error("Resource unavailable: {resource_id} - {reason}")]
    Unavailable {
        /// Resource identifier
        resource_id: String,
        /// Reason for unavailability
        reason: String,
        /// Whether the error is retryable
        retryable: bool,
    },

    /// Health check failed for the resource
    #[error("Health check failed for {resource_id} (attempt {attempt}): {reason}")]
    HealthCheckFailed {
        /// Resource identifier
        resource_id: String,
        /// Health check attempt number
        attempt: u32,
        /// Reason for health check failure
        reason: String,
    },

    /// Circuit breaker is open, preventing resource access
    #[error("Circuit breaker open for {resource_id}")]
    CircuitBreakerOpen {
        /// Resource identifier
        resource_id: String,
        /// Milliseconds until retry should be attempted
        retry_after_ms: Option<u64>,
    },

    /// Resource pool has been exhausted
    #[error(
        "Resource pool exhausted: {resource_id} ({current_size}/{max_size}, {waiters} waiting)"
    )]
    PoolExhausted {
        /// Resource identifier
        resource_id: String,
        /// Current number of resources in use
        current_size: usize,
        /// Maximum pool size
        max_size: usize,
        /// Number of waiters in queue
        waiters: usize,
    },

    /// A resource dependency has failed
    #[error("Dependency failure: {resource_id} depends on {dependency_id} - {reason}")]
    DependencyFailure {
        /// Resource identifier
        resource_id: String,
        /// Dependency identifier that failed
        dependency_id: String,
        /// Reason for dependency failure
        reason: String,
    },

    /// Circular dependency detected in resource graph
    #[error("Circular dependency detected: {cycle}")]
    CircularDependency {
        /// String representation of the dependency cycle
        cycle: String,
    },

    /// Invalid state transition attempted
    #[error("Invalid state transition for {resource_id}: {from} -> {to}")]
    InvalidStateTransition {
        /// Resource identifier
        resource_id: String,
        /// Current state
        from: String,
        /// Attempted target state
        to: String,
    },

    /// Resource initialization failed
    #[error("Resource initialization failed: {resource_id} - {reason}")]
    InitializationFailed {
        /// Resource identifier
        resource_id: String,
        /// Reason for initialization failure
        reason: String,
    },

    /// Resource cleanup/disposal failed
    #[error("Resource cleanup failed: {resource_id} - {reason}")]
    CleanupFailed {
        /// Resource identifier
        resource_id: String,
        /// Reason for cleanup failure
        reason: String,
    },

    /// Invalid resource configuration
    #[error("Invalid resource configuration: {resource_id} - {reason}")]
    InvalidConfiguration {
        /// Resource identifier
        resource_id: String,
        /// Reason why configuration is invalid
        reason: String,
    },

    /// Resource operation timed out
    #[error("Resource operation timed out: {resource_id} - {operation} after {timeout_ms}ms")]
    Timeout {
        /// Resource identifier
        resource_id: String,
        /// Operation that timed out
        operation: String,
        /// Timeout duration in milliseconds
        timeout_ms: u64,
    },

    /// Missing required credential for resource
    #[error("Missing credential '{credential_id}' for resource '{resource_id}'")]
    MissingCredential {
        /// Credential identifier
        credential_id: String,
        /// Resource identifier
        resource_id: String,
    },

    /// Resource is in invalid state
    #[error("Invalid resource state: {resource_id} - {reason}")]
    InvalidState {
        /// Resource identifier
        resource_id: String,
        /// Description of the invalid state
        reason: String,
    },

    /// Resource connection failed
    #[error("Resource connection failed: {resource_id} - {reason}")]
    ConnectionFailed {
        /// Resource identifier
        resource_id: String,
        /// Reason for connection failure
        reason: String,
    },

    /// Resource not found
    #[error("Resource not found: {resource_id}")]
    NotFound {
        /// Resource identifier
        resource_id: String,
    },

    /// Resource already exists
    #[error("Resource already exists: {resource_id}")]
    AlreadyExists {
        /// Resource identifier
        resource_id: String,
    },
}

impl ErrorCode for ResourceError {
    fn error_code(&self) -> &str {
        match self {
            ResourceError::Unavailable { .. } => codes::RESOURCE_UNAVAILABLE,
            ResourceError::HealthCheckFailed { .. } => codes::RESOURCE_HEALTH_CHECK_FAILED,
            ResourceError::CircuitBreakerOpen { .. } => codes::RESOURCE_CIRCUIT_BREAKER_OPEN,
            ResourceError::PoolExhausted { .. } => codes::RESOURCE_POOL_EXHAUSTED,
            ResourceError::DependencyFailure { .. } => codes::RESOURCE_DEPENDENCY_FAILURE,
            ResourceError::CircularDependency { .. } => codes::RESOURCE_CIRCULAR_DEPENDENCY,
            ResourceError::InvalidStateTransition { .. } => {
                codes::RESOURCE_INVALID_STATE_TRANSITION
            }
            ResourceError::InitializationFailed { .. } => codes::RESOURCE_INITIALIZATION_FAILED,
            ResourceError::CleanupFailed { .. } => codes::RESOURCE_CLEANUP_FAILED,
            ResourceError::InvalidConfiguration { .. } => codes::RESOURCE_INVALID_CONFIGURATION,
            ResourceError::Timeout { .. } => codes::RESOURCE_TIMEOUT,
            ResourceError::MissingCredential { .. } => codes::RESOURCE_MISSING_CREDENTIAL,
            ResourceError::InvalidState { .. } => codes::RESOURCE_INVALID_STATE,
            ResourceError::ConnectionFailed { .. } => codes::RESOURCE_CONNECTION_FAILED,
            ResourceError::NotFound { .. } => codes::RESOURCE_NOT_FOUND,
            ResourceError::AlreadyExists { .. } => codes::RESOURCE_ALREADY_EXISTS,
        }
    }

    fn error_category(&self) -> &'static str {
        codes::CATEGORY_RESOURCE
    }
}

impl ResourceError {
    /// Check if this error is retryable
    ///
    /// Determines if the operation that caused this error can be safely retried.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            ResourceError::Unavailable { retryable, .. } => *retryable,
            ResourceError::CircuitBreakerOpen { .. } => true,
            ResourceError::PoolExhausted { .. } => true,
            ResourceError::Timeout { .. } => true,
            ResourceError::ConnectionFailed { .. } => true,
            ResourceError::HealthCheckFailed { .. } => true,
            _ => false,
        }
    }

    /// Check if this is a configuration error
    #[must_use]
    pub fn is_config_error(&self) -> bool {
        matches!(
            self,
            ResourceError::InvalidConfiguration { .. }
                | ResourceError::CircularDependency { .. }
                | ResourceError::MissingCredential { .. }
        )
    }

    /// Check if this is a client error (4xx equivalent)
    #[must_use]
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            ResourceError::NotFound { .. }
                | ResourceError::AlreadyExists { .. }
                | ResourceError::InvalidConfiguration { .. }
                | ResourceError::InvalidStateTransition { .. }
                | ResourceError::CircularDependency { .. }
        )
    }

    /// Check if this is a server/infrastructure error (5xx equivalent)
    #[must_use]
    pub fn is_server_error(&self) -> bool {
        matches!(
            self,
            ResourceError::Unavailable { .. }
                | ResourceError::HealthCheckFailed { .. }
                | ResourceError::CircuitBreakerOpen { .. }
                | ResourceError::PoolExhausted { .. }
                | ResourceError::DependencyFailure { .. }
                | ResourceError::Timeout { .. }
                | ResourceError::ConnectionFailed { .. }
        )
    }

    /// Get the resource ID associated with this error
    #[must_use]
    pub fn resource_id(&self) -> Option<&str> {
        match self {
            ResourceError::Unavailable { resource_id, .. }
            | ResourceError::HealthCheckFailed { resource_id, .. }
            | ResourceError::CircuitBreakerOpen { resource_id, .. }
            | ResourceError::PoolExhausted { resource_id, .. }
            | ResourceError::DependencyFailure { resource_id, .. }
            | ResourceError::InvalidStateTransition { resource_id, .. }
            | ResourceError::InitializationFailed { resource_id, .. }
            | ResourceError::CleanupFailed { resource_id, .. }
            | ResourceError::InvalidConfiguration { resource_id, .. }
            | ResourceError::Timeout { resource_id, .. }
            | ResourceError::MissingCredential { resource_id, .. }
            | ResourceError::InvalidState { resource_id, .. }
            | ResourceError::ConnectionFailed { resource_id, .. }
            | ResourceError::NotFound { resource_id, .. }
            | ResourceError::AlreadyExists { resource_id, .. } => Some(resource_id),
            ResourceError::CircularDependency { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unavailable() {
        let err = ResourceError::Unavailable {
            resource_id: "db-pool".to_string(),
            reason: "maintenance".to_string(),
            retryable: true,
        };
        assert_eq!(err.error_code(), codes::RESOURCE_UNAVAILABLE);
        assert_eq!(err.error_category(), codes::CATEGORY_RESOURCE);
        assert!(err.is_retryable());
        assert!(err.is_server_error());
        assert!(!err.is_client_error());
        assert_eq!(err.resource_id(), Some("db-pool"));
    }

    #[test]
    fn test_circuit_breaker_open() {
        let err = ResourceError::CircuitBreakerOpen {
            resource_id: "api-client".to_string(),
            retry_after_ms: Some(5000),
        };
        assert_eq!(err.error_code(), codes::RESOURCE_CIRCUIT_BREAKER_OPEN);
        assert!(err.is_retryable());
        assert!(err.is_server_error());
    }

    #[test]
    fn test_pool_exhausted() {
        let err = ResourceError::PoolExhausted {
            resource_id: "worker-pool".to_string(),
            current_size: 10,
            max_size: 10,
            waiters: 5,
        };
        assert_eq!(err.error_code(), codes::RESOURCE_POOL_EXHAUSTED);
        assert!(err.is_retryable());
        assert!(err.is_server_error());
    }

    #[test]
    fn test_circular_dependency() {
        let err = ResourceError::CircularDependency {
            cycle: "A -> B -> C -> A".to_string(),
        };
        assert_eq!(err.error_code(), codes::RESOURCE_CIRCULAR_DEPENDENCY);
        assert!(!err.is_retryable());
        assert!(err.is_config_error());
        assert!(err.is_client_error());
        assert_eq!(err.resource_id(), None);
    }

    #[test]
    fn test_invalid_state_transition() {
        let err = ResourceError::InvalidStateTransition {
            resource_id: "workflow-1".to_string(),
            from: "stopped".to_string(),
            to: "running".to_string(),
        };
        assert_eq!(err.error_code(), codes::RESOURCE_INVALID_STATE_TRANSITION);
        assert!(!err.is_retryable());
        assert!(err.is_client_error());
    }

    #[test]
    fn test_not_found() {
        let err = ResourceError::NotFound {
            resource_id: "missing-resource".to_string(),
        };
        assert_eq!(err.error_code(), codes::RESOURCE_NOT_FOUND);
        assert!(!err.is_retryable());
        assert!(err.is_client_error());
    }

    #[test]
    fn test_error_display() {
        let err = ResourceError::Timeout {
            resource_id: "api-call".to_string(),
            operation: "fetch_data".to_string(),
            timeout_ms: 5000,
        };
        let display = format!("{}", err);
        assert!(display.contains("api-call"));
        assert!(display.contains("fetch_data"));
        assert!(display.contains("5000"));
    }
}
