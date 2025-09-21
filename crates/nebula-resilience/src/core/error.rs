//! Error types for resilience operations

use std::time::Duration;
use nebula_error::Error;

/// Errors that can occur during resilience operations
#[derive(Error, Debug, Clone)]
pub enum ResilienceError {
    /// Operation timed out
    #[error("Operation timed out after {duration:?}")]
    Timeout {
        /// Duration that was exceeded
        duration: Duration,
        /// Optional context about what timed out
        context: Option<String>,
    },

    /// Circuit breaker is open
    #[error("Circuit breaker is open (state: {state})")]
    CircuitBreakerOpen {
        /// Current circuit breaker state
        state: String,
        /// Time until next retry attempt
        retry_after: Option<Duration>,
    },

    /// Bulkhead is full
    #[error("Bulkhead is full (max_concurrency: {max_concurrency}, queued: {queued})")]
    BulkheadFull {
        /// Maximum concurrency limit
        max_concurrency: usize,
        /// Number of operations currently queued
        queued: usize,
    },

    /// Rate limit exceeded
    #[error("Rate limit exceeded (limit: {limit}/s, current: {current}/s)")]
    RateLimitExceeded {
        /// Time to wait before retry
        retry_after: Option<Duration>,
        /// Current rate limit
        limit: f64,
        /// Current request rate
        current: f64,
    },

    /// Retry limit exceeded
    #[error("Retry limit exceeded after {attempts} attempts (last error: {last_error:?})")]
    RetryLimitExceeded {
        /// Number of attempts made
        attempts: usize,
        /// Last error encountered
        last_error: Option<Box<ResilienceError>>,
    },

    /// Fallback operation failed
    #[error("Fallback operation failed: {reason}")]
    FallbackFailed {
        /// Reason for fallback failure
        reason: String,
        /// Original error that triggered fallback
        original_error: Option<Box<ResilienceError>>,
    },

    /// Operation was cancelled
    #[error("Operation was cancelled")]
    Cancelled,

    /// Invalid resilience configuration
    #[error("Invalid resilience configuration: {message}")]
    InvalidConfig {
        /// Error message
        message: String,
    },

    /// Wrapped Nebula error
    #[error("Nebula error: {0}")]
    Nebula(#[from] nebula_error::NebulaError),

    /// Custom error for extension
    #[error("Custom error: {message}")]
    Custom {
        message: String,
        retryable: bool,
    },
}

impl ResilienceError {
    /// Check if the error is retryable
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Timeout { .. } |
            Self::RateLimitExceeded { .. } => true,
            Self::CircuitBreakerOpen { .. } => false, // Don't retry when circuit is open
            Self::BulkheadFull { .. } => true, // Can retry after queue clears
            Self::Custom { retryable, .. } => *retryable,
            Self::Nebula(e) => {
                // Check if underlying Nebula error is retryable
                // This would depend on your nebula_error implementation
                false
            }
            _ => false,
        }
    }

    /// Check if the error is terminal (should not be retried)
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::InvalidConfig { .. } |
            Self::FallbackFailed { .. } |
            Self::Cancelled
        )
    }

    /// Get retry delay hint if available
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimitExceeded { retry_after, .. } |
            Self::CircuitBreakerOpen { retry_after, .. } => *retry_after,
            _ => None,
        }
    }

    /// Create a timeout error with context
    pub fn timeout_with_context(duration: Duration, context: impl Into<String>) -> Self {
        Self::Timeout {
            duration,
            context: Some(context.into()),
        }
    }

    /// Create a circuit breaker open error
    pub fn circuit_breaker_open(state: impl Into<String>, retry_after: Option<Duration>) -> Self {
        Self::CircuitBreakerOpen {
            state: state.into(),
            retry_after,
        }
    }

    /// Create a bulkhead full error
    #[must_use]
    pub const fn bulkhead_full(max_concurrency: usize, queued: usize) -> Self {
        Self::BulkheadFull { max_concurrency, queued }
    }

    /// Create a rate limit exceeded error
    #[must_use]
    pub const fn rate_limit_exceeded(limit: f64, current: f64, retry_after: Option<Duration>) -> Self {
        Self::RateLimitExceeded { retry_after, limit, current }
    }

    /// Create a retry limit exceeded error with last error
    pub fn retry_limit_exceeded_with_cause(attempts: usize, last_error: ResilienceError) -> Self {
        Self::RetryLimitExceeded {
            attempts,
            last_error: Some(Box::new(last_error)),
        }
    }

    /// Create a custom error
    pub fn custom(message: impl Into<String>, retryable: bool) -> Self {
        Self::Custom {
            message: message.into(),
            retryable,
        }
    }
}

/// Result type for resilience operations
pub type ResilienceResult<T> = Result<T, ResilienceError>;

/// Error classification for decision-making
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient errors that should be retried
    Transient,
    /// Resource exhaustion errors
    ResourceExhaustion,
    /// Configuration or programming errors
    Configuration,
    /// Permanent failures
    Permanent,
    /// Unknown error class
    Unknown,
}

impl ResilienceError {
    /// Classify the error for decision-making
    #[must_use]
    pub fn classify(&self) -> ErrorClass {
        match self {
            Self::Timeout { .. } => ErrorClass::Transient,
            Self::CircuitBreakerOpen { .. } => ErrorClass::ResourceExhaustion,
            Self::BulkheadFull { .. } => ErrorClass::ResourceExhaustion,
            Self::RateLimitExceeded { .. } => ErrorClass::ResourceExhaustion,
            Self::RetryLimitExceeded { .. } => ErrorClass::Permanent,
            Self::InvalidConfig { .. } => ErrorClass::Configuration,
            Self::FallbackFailed { .. } => ErrorClass::Permanent,
            Self::Cancelled => ErrorClass::Permanent,
            Self::Custom { retryable, .. } => {
                if *retryable {
                    ErrorClass::Transient
                } else {
                    ErrorClass::Permanent
                }
            }
            Self::Nebula(_) => ErrorClass::Unknown,
        }
    }
}