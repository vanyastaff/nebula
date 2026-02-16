//! Error types for resource management
use thiserror::Error;

/// Result type for resource operations
pub type Result<T> = std::result::Result<T, Error>;

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
