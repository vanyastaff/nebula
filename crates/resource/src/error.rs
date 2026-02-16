//! Error types for resource management
use thiserror::Error;

/// Result type for resource operations
pub type Result<T> = std::result::Result<T, Error>;

/// A single field validation failure.
#[derive(Debug, Clone)]
pub struct FieldViolation {
    /// The field name (e.g. "max_size").
    pub field: String,
    /// The constraint that was violated (e.g. "must be > 0").
    pub constraint: String,
    /// The actual value that failed (as a string representation).
    pub actual: String,
}

impl FieldViolation {
    /// Create a new field violation.
    pub fn new(
        field: impl Into<String>,
        constraint: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self {
            field: field.into(),
            constraint: constraint.into(),
            actual: actual.into(),
        }
    }
}

impl std::fmt::Display for FieldViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} (got {})",
            self.field, self.constraint, self.actual
        )
    }
}

/// Comprehensive error type for resource management operations
#[derive(Error, Debug)]
pub enum Error {
    /// Resource configuration is invalid
    #[error("Configuration error: {message}")]
    Configuration {
        /// The error message
        message: String,
        /// The invalid configuration value (if available)
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Resource initialization failed
    #[error("Initialization failed for resource '{resource_id}': {reason}")]
    Initialization {
        /// The resource identifier
        resource_id: String,
        /// The failure reason
        reason: String,
        /// The underlying error
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Resource is not available
    #[error("Resource '{resource_id}' is unavailable: {reason}")]
    Unavailable {
        /// The resource identifier
        resource_id: String,
        /// The unavailability reason
        reason: String,
        /// Whether the resource might become available later
        retryable: bool,
    },

    /// Health check failed
    #[error("Health check failed for resource '{resource_id}': {reason}")]
    HealthCheck {
        /// The resource identifier
        resource_id: String,
        /// The health check failure reason
        reason: String,
        /// The health check attempt number
        attempt: u32,
    },

    /// Required credential is missing
    #[error("Missing credential '{credential_id}' for resource '{resource_id}'")]
    MissingCredential {
        /// The credential identifier
        credential_id: String,
        /// The resource identifier
        resource_id: String,
    },

    /// Resource cleanup failed
    #[error("Cleanup failed for resource '{resource_id}': {reason}")]
    Cleanup {
        /// The resource identifier
        resource_id: String,
        /// The cleanup failure reason
        reason: String,
        /// The underlying error
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Operation timeout
    #[error("Operation timed out after {timeout_ms}ms for resource '{resource_id}'")]
    Timeout {
        /// The resource identifier
        resource_id: String,
        /// The timeout duration in milliseconds
        timeout_ms: u64,
        /// The operation that timed out
        operation: String,
    },

    /// Circuit breaker is open
    #[error("Circuit breaker is open for resource '{resource_id}'")]
    CircuitBreakerOpen {
        /// The resource identifier
        resource_id: String,
        /// When the circuit breaker will attempt to close
        retry_after_ms: Option<u64>,
    },

    /// Resource pool is exhausted
    #[error("Resource pool exhausted for '{resource_id}': {current_size}/{max_size} in use")]
    PoolExhausted {
        /// The resource identifier
        resource_id: String,
        /// Current pool size
        current_size: usize,
        /// Maximum pool size
        max_size: usize,
        /// Number of waiters in queue
        waiters: usize,
    },

    /// Resource dependency failure
    #[error("Dependency '{dependency_id}' failed for resource '{resource_id}': {reason}")]
    DependencyFailure {
        /// The resource identifier
        resource_id: String,
        /// The dependency identifier
        dependency_id: String,
        /// The failure reason
        reason: String,
    },

    /// Circular dependency detected
    #[error("Circular dependency detected: {cycle}")]
    CircularDependency {
        /// The dependency cycle as a string
        cycle: String,
    },

    /// Resource state error
    #[error("Invalid state transition for resource '{resource_id}': {from} -> {to}")]
    InvalidStateTransition {
        /// The resource identifier
        resource_id: String,
        /// The current state
        from: String,
        /// The attempted target state
        to: String,
    },

    /// One or more configuration fields failed validation.
    #[error("Validation error: {violations:?}")]
    Validation {
        /// Individual field validation failures.
        violations: Vec<FieldViolation>,
    },

    /// Generic internal error
    #[error("Internal error in resource '{resource_id}': {message}")]
    Internal {
        /// The resource identifier
        resource_id: String,
        /// The error message
        message: String,
        /// The underlying error
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl Error {
    /// Create a configuration error
    pub fn configuration<S: Into<String>>(message: S) -> Self {
        Self::Configuration {
            message: message.into(),
            source: None,
        }
    }

    /// Create a validation error from a list of field violations.
    pub fn validation(violations: Vec<FieldViolation>) -> Self {
        Self::Validation { violations }
    }

    /// Check if this error is retryable
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Unavailable { retryable, .. } => *retryable,
            Self::Timeout { .. } => true,
            Self::PoolExhausted { .. } => true,
            Self::CircuitBreakerOpen { .. } => true,
            _ => false,
        }
    }

    /// Get the resource ID associated with this error (if any)
    #[must_use]
    pub fn resource_id(&self) -> Option<&str> {
        match self {
            Self::Configuration { .. } => None,
            Self::CircularDependency { .. } => None,
            Self::Validation { .. } => None,
            Self::Initialization { resource_id, .. }
            | Self::Unavailable { resource_id, .. }
            | Self::HealthCheck { resource_id, .. }
            | Self::MissingCredential { resource_id, .. }
            | Self::Cleanup { resource_id, .. }
            | Self::Timeout { resource_id, .. }
            | Self::CircuitBreakerOpen { resource_id, .. }
            | Self::PoolExhausted { resource_id, .. }
            | Self::DependencyFailure { resource_id, .. }
            | Self::InvalidStateTransition { resource_id, .. }
            | Self::Internal { resource_id, .. } => Some(resource_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_has_no_resource_id() {
        let err = Error::configuration("bad config");
        assert!(err.resource_id().is_none());
        assert!(!err.is_retryable());
    }

    #[test]
    fn circular_dependency_has_no_resource_id() {
        let err = Error::CircularDependency {
            cycle: "a -> b -> a".to_string(),
        };
        assert!(err.resource_id().is_none());
        assert!(!err.is_retryable());
    }

    #[test]
    fn pool_exhausted_is_retryable_with_resource_id() {
        let err = Error::PoolExhausted {
            resource_id: "postgres".to_string(),
            current_size: 10,
            max_size: 10,
            waiters: 5,
        };
        assert_eq!(err.resource_id(), Some("postgres"));
        assert!(err.is_retryable());
    }

    #[test]
    fn timeout_is_retryable_with_resource_id() {
        let err = Error::Timeout {
            resource_id: "redis".to_string(),
            timeout_ms: 5000,
            operation: "connect".to_string(),
        };
        assert_eq!(err.resource_id(), Some("redis"));
        assert!(err.is_retryable());
    }

    #[test]
    fn circuit_breaker_open_is_retryable() {
        let err = Error::CircuitBreakerOpen {
            resource_id: "api".to_string(),
            retry_after_ms: Some(30000),
        };
        assert_eq!(err.resource_id(), Some("api"));
        assert!(err.is_retryable());
    }

    #[test]
    fn unavailable_retryable_depends_on_flag() {
        let retryable = Error::Unavailable {
            resource_id: "db".to_string(),
            reason: "overloaded".to_string(),
            retryable: true,
        };
        assert!(retryable.is_retryable());

        let not_retryable = Error::Unavailable {
            resource_id: "db".to_string(),
            reason: "not found".to_string(),
            retryable: false,
        };
        assert!(!not_retryable.is_retryable());
    }

    #[test]
    fn all_resource_id_variants_covered() {
        // Variants with resource_id
        let variants_with_id: Vec<Error> = vec![
            Error::Initialization {
                resource_id: "r".into(),
                reason: "fail".into(),
                source: None,
            },
            Error::Unavailable {
                resource_id: "r".into(),
                reason: "down".into(),
                retryable: false,
            },
            Error::HealthCheck {
                resource_id: "r".into(),
                reason: "timeout".into(),
                attempt: 1,
            },
            Error::MissingCredential {
                credential_id: "key".into(),
                resource_id: "r".into(),
            },
            Error::Cleanup {
                resource_id: "r".into(),
                reason: "fail".into(),
                source: None,
            },
            Error::Timeout {
                resource_id: "r".into(),
                timeout_ms: 1000,
                operation: "op".into(),
            },
            Error::CircuitBreakerOpen {
                resource_id: "r".into(),
                retry_after_ms: None,
            },
            Error::PoolExhausted {
                resource_id: "r".into(),
                current_size: 1,
                max_size: 1,
                waiters: 0,
            },
            Error::DependencyFailure {
                resource_id: "r".into(),
                dependency_id: "dep".into(),
                reason: "fail".into(),
            },
            Error::InvalidStateTransition {
                resource_id: "r".into(),
                from: "Ready".into(),
                to: "Created".into(),
            },
            Error::Internal {
                resource_id: "r".into(),
                message: "bug".into(),
                source: None,
            },
        ];

        for err in &variants_with_id {
            assert_eq!(
                err.resource_id(),
                Some("r"),
                "expected resource_id for {:?}",
                err
            );
        }

        // Variants without resource_id
        let variants_without_id: Vec<Error> = vec![
            Error::configuration("bad"),
            Error::CircularDependency {
                cycle: "a -> b".into(),
            },
            Error::validation(vec![FieldViolation::new("max_size", "must be > 0", "0")]),
        ];

        for err in &variants_without_id {
            assert!(
                err.resource_id().is_none(),
                "expected no resource_id for {:?}",
                err
            );
        }
    }

    #[test]
    fn validation_error_has_no_resource_id() {
        let err = Error::validation(vec![
            FieldViolation::new("max_size", "must be > 0", "0"),
            FieldViolation::new("min_size", "must be <= max_size", "5"),
        ]);
        assert!(err.resource_id().is_none());
        assert!(!err.is_retryable());
    }

    #[test]
    fn validation_error_display() {
        let err = Error::validation(vec![FieldViolation::new("max_size", "must be > 0", "0")]);
        let msg = err.to_string();
        assert!(msg.contains("Validation error"));
        assert!(msg.contains("max_size"));
    }

    #[test]
    fn field_violation_display() {
        let v = FieldViolation::new("max_size", "must be > 0", "0");
        assert_eq!(v.to_string(), "max_size: must be > 0 (got 0)");
    }

    #[test]
    fn validation_convenience_constructor() {
        let violations = vec![
            FieldViolation::new("a", "required", ""),
            FieldViolation::new("b", "too large", "999"),
        ];
        let err = Error::validation(violations);
        match &err {
            Error::Validation { violations } => {
                assert_eq!(violations.len(), 2);
                assert_eq!(violations[0].field, "a");
                assert_eq!(violations[1].field, "b");
            }
            other => panic!("expected Validation, got: {other:?}"),
        }
    }

    #[test]
    fn error_display_messages() {
        let err = Error::configuration("invalid max_size");
        assert!(err.to_string().contains("invalid max_size"));

        let err = Error::PoolExhausted {
            resource_id: "pg".to_string(),
            current_size: 5,
            max_size: 5,
            waiters: 3,
        };
        assert!(err.to_string().contains("pg"));
        assert!(err.to_string().contains("5/5"));
    }
}
