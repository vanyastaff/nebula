//! Error types for resilience operations

use std::time::Duration;
use thiserror::Error;

// ResilienceError can be converted to other error types as needed
// by implementing From traits in consuming crates

/// Core resilience errors
#[derive(Debug, Error)]
#[must_use = "ResilienceError should be returned or handled"]
pub enum ResilienceError {
    /// Operation timed out
    #[error("Operation timed out after {duration:?}{}", context.as_ref().map(|c| format!(" - {c}")).unwrap_or_default())]
    Timeout {
        /// Duration that was exceeded
        duration: Duration,
        /// Optional context about what timed out
        context: Option<String>,
    },

    /// Circuit breaker is open
    #[error("Circuit breaker is {state}{}", retry_after.map(|d| format!(" (retry after {d:?})")).unwrap_or_default())]
    CircuitBreakerOpen {
        /// Current circuit breaker state
        state: String,
        /// Time until next retry attempt
        retry_after: Option<Duration>,
    },

    /// Bulkhead is full
    #[error("Bulkhead full: max={max_concurrency}, queued={queued}")]
    BulkheadFull {
        /// Maximum concurrency limit
        max_concurrency: usize,
        /// Number of operations currently queued
        queued: usize,
    },

    /// Rate limit exceeded
    #[error("Rate limit exceeded: limit={limit}/s, current={current}/s{}", retry_after.map(|d| format!(" (retry after {d:?})")).unwrap_or_default())]
    RateLimitExceeded {
        /// Time to wait before retry
        retry_after: Option<Duration>,
        /// Current rate limit
        limit: f64,
        /// Current request rate
        current: f64,
    },

    /// Retry limit exceeded
    #[error("Retry limit exceeded after {attempts} attempts{}", last_error.as_ref().map(|e| format!(" - last error: {e}")).unwrap_or_default())]
    RetryLimitExceeded {
        /// Number of attempts made
        attempts: usize,
        /// Last error encountered
        last_error: Option<Box<ResilienceError>>,
    },

    /// Fallback operation failed
    #[error("Fallback failed: {reason}")]
    FallbackFailed {
        /// Reason for fallback failure
        reason: String,
        /// Original error that triggered fallback
        original_error: Option<Box<ResilienceError>>,
    },

    /// Operation was cancelled
    #[error("Operation cancelled{}", reason.as_ref().map(|r| format!(": {r}")).unwrap_or_default())]
    Cancelled {
        /// Cancellation reason
        reason: Option<String>,
    },

    /// Invalid configuration
    #[error("Invalid configuration: {message}")]
    InvalidConfig {
        /// Configuration error details
        message: String,
    },

    /// Custom error for extensions
    #[error("{message}")]
    Custom {
        /// Error message
        message: String,
        /// Whether this error is retryable
        retryable: bool,
        /// Error source
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl Clone for ResilienceError {
    fn clone(&self) -> Self {
        match self {
            Self::Timeout { duration, context } => Self::Timeout {
                duration: *duration,
                context: context.clone(),
            },
            Self::CircuitBreakerOpen { state, retry_after } => Self::CircuitBreakerOpen {
                state: state.clone(),
                retry_after: *retry_after,
            },
            Self::BulkheadFull {
                max_concurrency,
                queued,
            } => Self::BulkheadFull {
                max_concurrency: *max_concurrency,
                queued: *queued,
            },
            Self::RateLimitExceeded {
                retry_after,
                limit,
                current,
            } => Self::RateLimitExceeded {
                retry_after: *retry_after,
                limit: *limit,
                current: *current,
            },
            Self::RetryLimitExceeded {
                attempts,
                last_error,
            } => Self::RetryLimitExceeded {
                attempts: *attempts,
                last_error: last_error.clone(),
            },
            Self::FallbackFailed {
                reason,
                original_error,
            } => Self::FallbackFailed {
                reason: reason.clone(),
                original_error: original_error.clone(),
            },
            Self::Cancelled { reason } => Self::Cancelled {
                reason: reason.clone(),
            },
            Self::InvalidConfig { message } => Self::InvalidConfig {
                message: message.clone(),
            },
            Self::Custom {
                message,
                retryable,
                source: _,
            } => Self::Custom {
                message: message.clone(),
                retryable: *retryable,
                source: None, // Can't clone trait objects, so we lose the source
            },
        }
    }
}

/// Error classification for decision making
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// Create a custom error (for testing/benchmarking)
    #[must_use]
    pub fn custom(message: impl Into<String>) -> Self {
        Self::Custom {
            message: message.into(),
            retryable: true,
            source: None,
        }
    }

    /// Create a timeout error
    #[must_use]
    pub fn timeout(duration: Duration) -> Self {
        Self::Timeout {
            duration,
            context: Some("Operation timed out".to_string()),
        }
    }

    /// Create a circuit breaker open error
    pub fn circuit_breaker_open(state: impl Into<String>) -> Self {
        Self::CircuitBreakerOpen {
            state: state.into(),
            retry_after: None,
        }
    }

    /// Create a bulkhead full error
    #[must_use]
    pub fn bulkhead_full(max_concurrency: usize) -> Self {
        Self::BulkheadFull {
            max_concurrency,
            queued: 0, // Default
        }
    }

    /// Create a retry limit exceeded error with cause
    #[must_use]
    pub fn retry_limit_exceeded_with_cause(attempts: usize, last_error: Option<Box<Self>>) -> Self {
        Self::RetryLimitExceeded {
            attempts,
            last_error,
        }
    }

    /// Classify the error for decision making
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
            Self::Cancelled { .. } => ErrorClass::Permanent,
            Self::Custom { retryable, .. } => {
                if *retryable {
                    ErrorClass::Transient
                } else {
                    ErrorClass::Permanent
                }
            }
        }
    }

    /// Check if the error is retryable
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.classify(),
            ErrorClass::Transient | ErrorClass::ResourceExhaustion
        )
    }

    /// Check if the error is terminal
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.classify(),
            ErrorClass::Permanent | ErrorClass::Configuration
        )
    }

    /// Get retry delay hint if available
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimitExceeded { retry_after, .. }
            | Self::CircuitBreakerOpen { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// Context for errors
#[derive(Debug, Clone)]
#[must_use = "ErrorContext should be used to provide error context"]
pub struct ErrorContext {
    /// Service name
    pub service: String,
    /// Operation name
    pub operation: String,
    /// Additional metadata
    pub metadata: std::collections::HashMap<String, String>,
    /// Timestamp
    pub timestamp: std::time::Instant,
}

impl ErrorContext {
    /// Create new error context
    pub fn new(service: impl Into<String>, operation: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            operation: operation.into(),
            metadata: std::collections::HashMap::new(),
            timestamp: std::time::Instant::now(),
        }
    }

    /// Add metadata
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ResilienceError can be converted to other error types as needed
// by implementing From traits in consuming crates

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let error = ResilienceError::timeout(Duration::from_secs(5));
        let display = error.to_string();
        assert!(display.contains("5s"), "Expected duration in display: {}", display);
    }

    #[test]
    fn test_error_source_chain() {
        let inner = ResilienceError::timeout(Duration::from_millis(100));
        let outer = ResilienceError::RetryLimitExceeded {
            attempts: 3,
            last_error: Some(Box::new(inner)),
        };

        // Verify error chain works
        assert!(outer.to_string().contains("3 attempts"));
    }

    #[test]
    fn test_error_classification() {
        assert_eq!(ResilienceError::timeout(Duration::from_secs(1)).classify(), ErrorClass::Transient);
        assert_eq!(ResilienceError::circuit_breaker_open("open").classify(), ErrorClass::ResourceExhaustion);
        assert_eq!(ResilienceError::InvalidConfig { message: "bad".into() }.classify(), ErrorClass::Configuration);
    }
}
