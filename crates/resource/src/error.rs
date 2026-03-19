//! Error types for resource management.
//!
//! All resource-level errors carry a [`nebula_core::ResourceKey`] where
//! applicable, ensuring a clear distinction between logical keys and
//! instance identifiers.
use nebula_core::ResourceKey;
use thiserror::Error;

/// Result type for resource operations.
pub type ResourceResult<T> = std::result::Result<T, Error>;

/// Backward-compatible result alias for resource operations.
pub type Result<T> = ResourceResult<T>;

/// High-level error category used by callers (`action`, `runtime`) to decide policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Temporary failure that may succeed when retried.
    Retryable,
    /// Permanent failure for the current operation.
    Fatal,
    /// Input/contract/config validation failure.
    Validation,
}

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
#[non_exhaustive]
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
    #[error("Initialization failed for resource '{resource_key}': {reason}")]
    Initialization {
        /// The resource key
        resource_key: ResourceKey,
        /// The failure reason
        reason: String,
        /// The underlying error
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Resource is not available
    #[error("Resource '{resource_key}' is unavailable: {reason}")]
    Unavailable {
        /// The resource key
        resource_key: ResourceKey,
        /// The unavailability reason
        reason: String,
        /// Whether the resource might become available later
        retryable: bool,
    },

    /// Health check failed
    #[error("Health check failed for resource '{resource_key}': {reason}")]
    HealthCheck {
        /// The resource key
        resource_key: ResourceKey,
        /// The health check failure reason
        reason: String,
        /// The health check attempt number
        attempt: u32,
    },

    /// Resource cleanup failed
    #[error("Cleanup failed for resource '{resource_key}': {reason}")]
    Cleanup {
        /// The resource key
        resource_key: ResourceKey,
        /// The cleanup failure reason
        reason: String,
        /// The underlying error
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Operation timeout
    #[error("Operation timed out after {timeout_ms}ms for resource '{resource_key}'")]
    Timeout {
        /// The resource key
        resource_key: ResourceKey,
        /// The timeout duration in milliseconds
        timeout_ms: u64,
        /// The operation that timed out
        operation: String,
    },

    /// Circuit breaker is open — the resource has been temporarily blocked
    /// due to repeated failures.
    #[error(
        "Circuit breaker is open for resource '{resource_key}' operation '{operation}' (retry_after={retry_after:?})"
    )]
    CircuitBreakerOpen {
        /// The resource key
        resource_key: ResourceKey,
        /// Operation name.
        operation: &'static str,
        /// When the circuit breaker allows a new probe.
        retry_after: Option<std::time::Duration>,
    },

    /// Resource pool is exhausted
    #[error("Resource pool exhausted for '{resource_key}': {current_size}/{max_size} in use")]
    PoolExhausted {
        /// The resource key
        resource_key: ResourceKey,
        /// Current pool size
        current_size: usize,
        /// Maximum pool size
        max_size: usize,
        /// Number of waiters in queue
        waiters: usize,
    },

    /// Resource dependency failure
    #[error("Dependency '{dependency_id}' failed for resource '{resource_key}': {reason}")]
    DependencyFailure {
        /// The resource key
        resource_key: ResourceKey,
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
    #[error("Invalid state transition for resource '{resource_key}': {from} -> {to}")]
    InvalidStateTransition {
        /// The resource key
        resource_key: ResourceKey,
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
    #[error("Internal error in resource '{resource_key}': {message}")]
    Internal {
        /// The resource key
        resource_key: ResourceKey,
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

    /// Check if this error is retryable.
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

    /// Classify the error into a stable, policy-friendly category.
    ///
    /// `Configuration` and `Validation` are treated as `Validation`.
    /// Other errors are classified by retryability.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Configuration { .. } | Self::Validation { .. } => ErrorCategory::Validation,
            _ if self.is_retryable() => ErrorCategory::Retryable,
            _ => ErrorCategory::Fatal,
        }
    }

    /// Check if this error is a validation failure.
    #[must_use]
    pub fn is_validation(&self) -> bool {
        self.category() == ErrorCategory::Validation
    }

    /// Check if this error is fatal (non-retryable, non-validation).
    #[must_use]
    pub fn is_fatal(&self) -> bool {
        self.category() == ErrorCategory::Fatal
    }

    /// Retry hint for backoff-aware callers.
    #[must_use]
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::CircuitBreakerOpen { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// The pool operation that was rejected by an open circuit breaker.
    ///
    /// Returns `Some("create")` or `Some("recycle")` for [`Error::CircuitBreakerOpen`],
    /// and `None` for all other variants.
    #[must_use]
    pub fn operation(&self) -> Option<&'static str> {
        match self {
            Self::CircuitBreakerOpen { operation, .. } => Some(operation),
            _ => None,
        }
    }

    /// Get the resource key associated with this error (if any).
    #[must_use]
    pub fn resource_key(&self) -> Option<&ResourceKey> {
        match self {
            Self::Configuration { .. } => None,
            Self::CircularDependency { .. } => None,
            Self::Validation { .. } => None,
            Self::Initialization { resource_key, .. }
            | Self::Unavailable { resource_key, .. }
            | Self::HealthCheck { resource_key, .. }
            | Self::Cleanup { resource_key, .. }
            | Self::Timeout { resource_key, .. }
            | Self::CircuitBreakerOpen { resource_key, .. }
            | Self::PoolExhausted { resource_key, .. }
            | Self::DependencyFailure { resource_key, .. }
            | Self::InvalidStateTransition { resource_key, .. }
            | Self::Internal { resource_key, .. } => Some(resource_key),
        }
    }
}

impl nebula_resilience::retryable::Retryable for Error {
    fn is_retryable(&self) -> bool {
        Error::is_retryable(self)
    }

    fn retry_delay(&self) -> std::time::Duration {
        self.retry_after()
            .unwrap_or(std::time::Duration::from_millis(100))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::resource_key;

    #[test]
    fn configuration_has_no_resource_id() {
        let err = Error::configuration("bad config");
        assert!(err.resource_key().is_none());
        assert!(!err.is_retryable());
    }

    #[test]
    fn circular_dependency_has_no_resource_id() {
        let err = Error::CircularDependency {
            cycle: "a -> b -> a".to_string(),
        };
        assert!(err.resource_key().is_none());
        assert!(!err.is_retryable());
    }

    #[test]
    fn pool_exhausted_is_retryable_with_resource_id() {
        let key = resource_key!("postgres");
        let err = Error::PoolExhausted {
            resource_key: key.clone(),
            current_size: 10,
            max_size: 10,
            waiters: 5,
        };
        assert_eq!(err.resource_key(), Some(&key));
        assert!(err.is_retryable());
    }

    #[test]
    fn timeout_is_retryable_with_resource_id() {
        let key = resource_key!("redis");
        let err = Error::Timeout {
            resource_key: key.clone(),
            timeout_ms: 5000,
            operation: "connect".to_string(),
        };
        assert_eq!(err.resource_key(), Some(&key));
        assert!(err.is_retryable());
    }

    #[test]
    fn circuit_breaker_open_is_retryable() {
        let key = resource_key!("api");
        let err = Error::CircuitBreakerOpen {
            resource_key: key.clone(),
            operation: "create",
            retry_after: Some(std::time::Duration::from_secs(30)),
        };
        assert_eq!(err.resource_key(), Some(&key));
        assert!(err.is_retryable());
        assert_eq!(err.retry_after(), Some(std::time::Duration::from_secs(30)));
    }

    #[test]
    fn unavailable_retryable_depends_on_flag() {
        let key = resource_key!("db");
        let retryable = Error::Unavailable {
            resource_key: key.clone(),
            reason: "overloaded".to_string(),
            retryable: true,
        };
        assert!(retryable.is_retryable());

        let not_retryable = Error::Unavailable {
            resource_key: key,
            reason: "not found".to_string(),
            retryable: false,
        };
        assert!(!not_retryable.is_retryable());
    }

    #[test]
    fn all_resource_id_variants_covered() {
        // Variants with resource_id
        let key = resource_key!("r");
        let variants_with_id: Vec<Error> = vec![
            Error::Initialization {
                resource_key: key.clone(),
                reason: "fail".into(),
                source: None,
            },
            Error::Unavailable {
                resource_key: key.clone(),
                reason: "down".into(),
                retryable: false,
            },
            Error::HealthCheck {
                resource_key: key.clone(),
                reason: "timeout".into(),
                attempt: 1,
            },
            Error::Cleanup {
                resource_key: key.clone(),
                reason: "fail".into(),
                source: None,
            },
            Error::Timeout {
                resource_key: key.clone(),
                timeout_ms: 1000,
                operation: "op".into(),
            },
            Error::CircuitBreakerOpen {
                resource_key: key.clone(),
                operation: "create",
                retry_after: None,
            },
            Error::PoolExhausted {
                resource_key: key.clone(),
                current_size: 1,
                max_size: 1,
                waiters: 0,
            },
            Error::DependencyFailure {
                resource_key: key.clone(),
                dependency_id: "dep".into(),
                reason: "fail".into(),
            },
            Error::InvalidStateTransition {
                resource_key: key.clone(),
                from: "Ready".into(),
                to: "Created".into(),
            },
            Error::Internal {
                resource_key: key,
                message: "bug".into(),
                source: None,
            },
        ];

        for err in &variants_with_id {
            assert_eq!(
                err.resource_key().map(ResourceKey::as_ref),
                Some("r"),
                "expected resource_key for {:?}",
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
                err.resource_key().is_none(),
                "expected no resource_key for {:?}",
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
        assert!(err.resource_key().is_none());
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

        let key = resource_key!("pg");
        let err = Error::PoolExhausted {
            resource_key: key,
            current_size: 5,
            max_size: 5,
            waiters: 3,
        };
        assert!(err.to_string().contains("pg"));
        assert!(err.to_string().contains("5/5"));
    }

    #[test]
    fn category_validation_for_configuration_and_validation_errors() {
        let cfg = Error::configuration("bad config");
        assert_eq!(cfg.category(), ErrorCategory::Validation);
        assert!(cfg.is_validation());

        let validation = Error::validation(vec![FieldViolation::new("x", "required", "")]);
        assert_eq!(validation.category(), ErrorCategory::Validation);
        assert!(validation.is_validation());
    }

    #[test]
    fn category_retryable_for_retryable_errors() {
        let key = resource_key!("retryable");
        let err = Error::PoolExhausted {
            resource_key: key,
            current_size: 2,
            max_size: 2,
            waiters: 1,
        };
        assert_eq!(err.category(), ErrorCategory::Retryable);
        assert!(!err.is_fatal());
    }

    #[test]
    fn category_fatal_for_non_retryable_operational_errors() {
        let key = resource_key!("fatal");
        let err = Error::Unavailable {
            resource_key: key,
            reason: "missing registration".into(),
            retryable: false,
        };
        assert_eq!(err.category(), ErrorCategory::Fatal);
        assert!(err.is_fatal());
    }

    #[test]
    fn circuit_breaker_open_operation_returns_op_name() {
        let key = resource_key!("cache");
        for op in ["create", "recycle"] {
            let err = Error::CircuitBreakerOpen {
                resource_key: key.clone(),
                operation: op,
                retry_after: None,
            };
            assert_eq!(err.operation(), Some(op), "operation should match for {op}");
        }
        // Any other variant must return None.
        assert_eq!(Error::configuration("bad").operation(), None);
    }
}
