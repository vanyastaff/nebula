//! Error types for resilience operations

use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during resilience operations
#[derive(Error, Debug, Clone)]
pub enum ResilienceError {
    /// Operation timed out
    #[error("Operation timed out after {duration:?}")]
    Timeout {
        /// Duration that was exceeded
        duration: Duration,
    },

    /// Circuit breaker is open
    #[error("Circuit breaker is open (state: {state:?})")]
    CircuitBreakerOpen {
        /// Current circuit breaker state
        state: String,
    },

    /// Bulkhead is full
    #[error("Bulkhead is full (max_concurrency: {max_concurrency})")]
    BulkheadFull {
        /// Maximum concurrency limit
        max_concurrency: usize,
    },

    /// Retry limit exceeded
    #[error("Retry limit exceeded after {attempts} attempts")]
    RetryLimitExceeded {
        /// Number of attempts made
        attempts: usize,
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
}

impl ResilienceError {
    /// Check if the error is retryable
    #[must_use] pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::CircuitBreakerOpen { .. })
    }

    /// Check if the error is terminal (should not be retried)
    #[must_use] pub fn is_terminal(&self) -> bool {
        matches!(self, Self::BulkheadFull { .. } | Self::InvalidConfig { .. })
    }

    /// Create a timeout error
    #[must_use] pub fn timeout(duration: Duration) -> Self {
        Self::Timeout { duration }
    }

    /// Create a circuit breaker open error
    pub fn circuit_breaker_open(state: impl Into<String>) -> Self {
        Self::CircuitBreakerOpen {
            state: state.into(),
        }
    }

    /// Create a bulkhead full error
    #[must_use] pub fn bulkhead_full(max_concurrency: usize) -> Self {
        Self::BulkheadFull { max_concurrency }
    }

    /// Create a retry limit exceeded error
    #[must_use] pub fn retry_limit_exceeded(attempts: usize) -> Self {
        Self::RetryLimitExceeded { attempts }
    }

    /// Create an invalid config error
    pub fn invalid_config(message: impl Into<String>) -> Self {
        Self::InvalidConfig {
            message: message.into(),
        }
    }
}

/// Result type for resilience operations
pub type ResilienceResult<T> = Result<T, ResilienceError>;
