//! Error types for resource management


use nebula_error::NebulaError;
use thiserror::Error;

/// Result type for resource operations
pub type ResourceResult<T> = Result<T, ResourceError>;

/// Comprehensive error type for resource management operations
#[derive(Error, Debug)]
pub enum ResourceError {
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

impl ResourceError {
    /// Create a configuration error
    pub fn configuration<S: Into<String>>(message: S) -> Self {
        Self::Configuration {
            message: message.into(),
            source: None,
        }
    }

    /// Create a configuration error with source
    pub fn configuration_with_source<S: Into<String>, E>(message: S, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Configuration {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create an initialization error
    pub fn initialization<S1: Into<String>, S2: Into<String>>(resource_id: S1, reason: S2) -> Self {
        Self::Initialization {
            resource_id: resource_id.into(),
            reason: reason.into(),
            source: None,
        }
    }

    /// Create an initialization error with source
    pub fn initialization_with_source<S1: Into<String>, S2: Into<String>, E>(
        resource_id: S1,
        reason: S2,
        source: E,
    ) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Initialization {
            resource_id: resource_id.into(),
            reason: reason.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create an unavailable error
    pub fn unavailable<S1: Into<String>, S2: Into<String>>(
        resource_id: S1,
        reason: S2,
        retryable: bool,
    ) -> Self {
        Self::Unavailable {
            resource_id: resource_id.into(),
            reason: reason.into(),
            retryable,
        }
    }

    /// Create a health check error
    pub fn health_check<S1: Into<String>, S2: Into<String>>(
        resource_id: S1,
        reason: S2,
        attempt: u32,
    ) -> Self {
        Self::HealthCheck {
            resource_id: resource_id.into(),
            reason: reason.into(),
            attempt,
        }
    }

    /// Create a missing credential error
    pub fn missing_credential<S1: Into<String>, S2: Into<String>>(
        credential_id: S1,
        resource_id: S2,
    ) -> Self {
        Self::MissingCredential {
            credential_id: credential_id.into(),
            resource_id: resource_id.into(),
        }
    }

    /// Create a cleanup error
    pub fn cleanup<S1: Into<String>, S2: Into<String>>(resource_id: S1, reason: S2) -> Self {
        Self::Cleanup {
            resource_id: resource_id.into(),
            reason: reason.into(),
            source: None,
        }
    }

    /// Create a timeout error
    pub fn timeout<S1: Into<String>, S2: Into<String>>(
        resource_id: S1,
        timeout_ms: u64,
        operation: S2,
    ) -> Self {
        Self::Timeout {
            resource_id: resource_id.into(),
            timeout_ms,
            operation: operation.into(),
        }
    }

    /// Create a circuit breaker open error
    pub fn circuit_breaker_open<S: Into<String>>(
        resource_id: S,
        retry_after_ms: Option<u64>,
    ) -> Self {
        Self::CircuitBreakerOpen {
            resource_id: resource_id.into(),
            retry_after_ms,
        }
    }

    /// Create a pool exhausted error
    pub fn pool_exhausted<S: Into<String>>(
        resource_id: S,
        current_size: usize,
        max_size: usize,
        waiters: usize,
    ) -> Self {
        Self::PoolExhausted {
            resource_id: resource_id.into(),
            current_size,
            max_size,
            waiters,
        }
    }

    /// Create a dependency failure error
    pub fn dependency_failure<S1: Into<String>, S2: Into<String>, S3: Into<String>>(
        resource_id: S1,
        dependency_id: S2,
        reason: S3,
    ) -> Self {
        Self::DependencyFailure {
            resource_id: resource_id.into(),
            dependency_id: dependency_id.into(),
            reason: reason.into(),
        }
    }

    /// Create a circular dependency error
    pub fn circular_dependency<S: Into<String>>(cycle: S) -> Self {
        Self::CircularDependency {
            cycle: cycle.into(),
        }
    }

    /// Create an invalid state transition error
    pub fn invalid_state_transition<S1: Into<String>, S2: Into<String>, S3: Into<String>>(
        resource_id: S1,
        from: S2,
        to: S3,
    ) -> Self {
        Self::InvalidStateTransition {
            resource_id: resource_id.into(),
            from: from.into(),
            to: to.into(),
        }
    }

    /// Create an internal error
    pub fn internal<S1: Into<String>, S2: Into<String>>(resource_id: S1, message: S2) -> Self {
        Self::Internal {
            resource_id: resource_id.into(),
            message: message.into(),
            source: None,
        }
    }

    /// Check if this error is retryable
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
    pub fn resource_id(&self) -> Option<&str> {
        match self {
            Self::Configuration { .. } => None,
            Self::CircularDependency { .. } => None,
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

// Integration with nebula-error
impl From<NebulaError> for ResourceError {
    fn from(error: NebulaError) -> Self {
        Self::Internal {
            resource_id: "unknown".to_string(),
            message: error.to_string(),
            source: Some(Box::new(error)),
        }
    }
}

impl From<ResourceError> for NebulaError {
    fn from(error: ResourceError) -> Self {
        // Convert ResourceError to appropriate NebulaError System variant
        use nebula_error::kinds::system::SystemError;

        let sys_error = SystemError::ResourceExhausted {
            resource: format!("{}: {}", error.resource_id().unwrap_or("unknown"), error),
        };

        NebulaError::new(nebula_error::ErrorKind::System(sys_error))
    }
}
