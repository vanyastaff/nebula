//! Server error types (5xx equivalent)
//!
//! These errors are typically caused by internal server issues, service failures,
//! or other server-side problems. Many of these may be retryable.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

use crate::core::traits::{ErrorCode, RetryableError};

/// Server-side error variants
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ServerError {
    /// Internal server error
    #[error("Internal error: {message}")]
    Internal { message: String },

    /// Service unavailable
    #[error("Service unavailable: {service} - {reason}")]
    ServiceUnavailable { service: String, reason: String },

    /// Service temporarily overloaded
    #[error("Service overloaded: {service} - {message}")]
    ServiceOverloaded { service: String, message: String },

    /// Configuration error
    #[error("Configuration error: {message}")]
    Configuration { message: String },

    /// Dependency failure
    #[error("Dependency failure: {dependency} - {reason}")]
    DependencyFailure { dependency: String, reason: String },

    /// Maintenance mode
    #[error("Service in maintenance mode: {message}")]
    Maintenance { message: String },

    /// Feature not implemented
    #[error("Feature not implemented: {feature}")]
    NotImplemented { feature: String },

    /// Version mismatch
    #[error("Version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: String, actual: String },
}

impl RetryableError for ServerError {
    fn is_retryable(&self) -> bool {
        match self {
            ServerError::Internal { .. } => false, // Internal errors usually need fixing
            ServerError::ServiceUnavailable { .. } => true,
            ServerError::ServiceOverloaded { .. } => true,
            ServerError::Configuration { .. } => false, // Config errors need manual fixing
            ServerError::DependencyFailure { .. } => true, // Dependencies might recover
            ServerError::Maintenance { .. } => true,    // Maintenance should end eventually
            ServerError::NotImplemented { .. } => false, // Won't be fixed by retrying
            ServerError::VersionMismatch { .. } => false, // Version issues need manual fixing
        }
    }

    fn retry_delay(&self) -> Option<Duration> {
        match self {
            ServerError::ServiceUnavailable { .. } => Some(Duration::from_secs(5)),
            ServerError::ServiceOverloaded { .. } => Some(Duration::from_secs(10)),
            ServerError::DependencyFailure { .. } => Some(Duration::from_secs(3)),
            ServerError::Maintenance { .. } => Some(Duration::from_secs(30)),
            _ => None,
        }
    }
}

impl ErrorCode for ServerError {
    fn error_code(&self) -> &str {
        match self {
            ServerError::Internal { .. } => "INTERNAL_ERROR",
            ServerError::ServiceUnavailable { .. } => "SERVICE_UNAVAILABLE_ERROR",
            ServerError::ServiceOverloaded { .. } => "SERVICE_OVERLOADED_ERROR",
            ServerError::Configuration { .. } => "CONFIGURATION_ERROR",
            ServerError::DependencyFailure { .. } => "DEPENDENCY_FAILURE_ERROR",
            ServerError::Maintenance { .. } => "MAINTENANCE_ERROR",
            ServerError::NotImplemented { .. } => "NOT_IMPLEMENTED_ERROR",
            ServerError::VersionMismatch { .. } => "VERSION_MISMATCH_ERROR",
        }
    }

    fn error_category(&self) -> &str {
        "SERVER"
    }
}

impl ServerError {
    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Create a service unavailable error
    pub fn service_unavailable(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::ServiceUnavailable {
            service: service.into(),
            reason: reason.into(),
        }
    }

    /// Create a service overloaded error
    pub fn service_overloaded(service: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ServiceOverloaded {
            service: service.into(),
            message: message.into(),
        }
    }

    /// Create a configuration error
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration {
            message: message.into(),
        }
    }

    /// Create a dependency failure error
    pub fn dependency_failure(dependency: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::DependencyFailure {
            dependency: dependency.into(),
            reason: reason.into(),
        }
    }

    /// Create a maintenance error
    pub fn maintenance(message: impl Into<String>) -> Self {
        Self::Maintenance {
            message: message.into(),
        }
    }

    /// Create a not implemented error
    pub fn not_implemented(feature: impl Into<String>) -> Self {
        Self::NotImplemented {
            feature: feature.into(),
        }
    }

    /// Create a version mismatch error
    pub fn version_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::VersionMismatch {
            expected: expected.into(),
            actual: actual.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_error_creation() {
        let internal_error = ServerError::internal("Database connection failed");
        assert_eq!(internal_error.error_code(), "INTERNAL_ERROR");
        assert!(!internal_error.is_retryable());

        let service_error =
            ServerError::service_unavailable("database", "connection pool exhausted");
        assert_eq!(service_error.error_code(), "SERVICE_UNAVAILABLE_ERROR");
        assert!(service_error.is_retryable());

        let maintenance_error = ServerError::maintenance("Scheduled maintenance in progress");
        assert_eq!(maintenance_error.error_code(), "MAINTENANCE_ERROR");
        assert!(maintenance_error.is_retryable());
    }

    #[test]
    fn test_server_error_display() {
        let internal_error = ServerError::internal("Database connection failed");
        assert_eq!(
            internal_error.to_string(),
            "Internal error: Database connection failed"
        );

        let service_error =
            ServerError::service_unavailable("database", "connection pool exhausted");
        assert_eq!(
            service_error.to_string(),
            "Service unavailable: database - connection pool exhausted"
        );

        let version_error = ServerError::version_mismatch("v2.0", "v1.0");
        assert_eq!(
            version_error.to_string(),
            "Version mismatch: expected v2.0, got v1.0"
        );
    }

    #[test]
    fn test_retry_behavior() {
        let internal_error = ServerError::internal("Database error");
        assert!(!internal_error.is_retryable());
        assert_eq!(internal_error.retry_delay(), None);

        let service_error = ServerError::service_unavailable("database", "overloaded");
        assert!(service_error.is_retryable());
        assert_eq!(service_error.retry_delay(), Some(Duration::from_secs(5)));

        let overloaded_error = ServerError::service_overloaded("api", "too many requests");
        assert!(overloaded_error.is_retryable());
        assert_eq!(
            overloaded_error.retry_delay(),
            Some(Duration::from_secs(10))
        );
    }
}
