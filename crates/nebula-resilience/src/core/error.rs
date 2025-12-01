//! Error types for resilience operations

use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

// ResilienceError can be converted to other error types as needed
// by implementing From traits in consuming crates

/// Core resilience errors
#[derive(Debug)]
#[must_use = "ResilienceError should be returned or handled"]
pub enum ResilienceError {
    /// Operation timed out
    Timeout {
        /// Duration that was exceeded
        duration: Duration,
        /// Optional context about what timed out
        context: Option<String>,
    },

    /// Circuit breaker is open
    CircuitBreakerOpen {
        /// Current circuit breaker state
        state: String,
        /// Time until next retry attempt
        retry_after: Option<Duration>,
    },

    /// Bulkhead is full
    BulkheadFull {
        /// Maximum concurrency limit
        max_concurrency: usize,
        /// Number of operations currently queued
        queued: usize,
    },

    /// Rate limit exceeded
    RateLimitExceeded {
        /// Time to wait before retry
        retry_after: Option<Duration>,
        /// Current rate limit
        limit: f64,
        /// Current request rate
        current: f64,
    },

    /// Retry limit exceeded
    RetryLimitExceeded {
        /// Number of attempts made
        attempts: usize,
        /// Last error encountered
        last_error: Option<Box<ResilienceError>>,
    },

    /// Fallback operation failed
    FallbackFailed {
        /// Reason for fallback failure
        reason: String,
        /// Original error that triggered fallback
        original_error: Option<Box<ResilienceError>>,
    },

    /// Operation was cancelled
    Cancelled {
        /// Cancellation reason
        reason: Option<String>,
    },

    /// Invalid configuration
    InvalidConfig {
        /// Configuration error details
        message: String,
    },

    /// Custom error for extensions
    Custom {
        /// Error message
        message: String,
        /// Whether this error is retryable
        retryable: bool,
        /// Error source
        source: Option<Box<dyn StdError + Send + Sync>>,
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

impl fmt::Display for ResilienceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { duration, context } => {
                write!(f, "Operation timed out after {duration:?}")?;
                if let Some(ctx) = context {
                    write!(f, " - {ctx}")?;
                }
                Ok(())
            }
            Self::CircuitBreakerOpen { state, retry_after } => {
                write!(f, "Circuit breaker is {state} ")?;
                if let Some(duration) = retry_after {
                    write!(f, "(retry after {duration:?})")?;
                }
                Ok(())
            }
            Self::BulkheadFull {
                max_concurrency,
                queued,
            } => {
                write!(f, "Bulkhead full: max={max_concurrency}, queued={queued}")
            }
            Self::RateLimitExceeded {
                limit,
                current,
                retry_after,
            } => {
                write!(
                    f,
                    "Rate limit exceeded: limit={limit}/s, current={current}/s"
                )?;
                if let Some(duration) = retry_after {
                    write!(f, " (retry after {duration:?})")?;
                }
                Ok(())
            }
            Self::RetryLimitExceeded {
                attempts,
                last_error,
            } => {
                write!(f, "Retry limit exceeded after {attempts} attempts")?;
                if let Some(err) = last_error {
                    write!(f, " - last error: {err}")?;
                }
                Ok(())
            }
            Self::FallbackFailed { reason, .. } => {
                write!(f, "Fallback failed: {reason}")
            }
            Self::Cancelled { reason } => {
                write!(f, "Operation cancelled")?;
                if let Some(r) = reason {
                    write!(f, ": {r}")?;
                }
                Ok(())
            }
            Self::InvalidConfig { message } => {
                write!(f, "Invalid configuration: {message}")
            }
            Self::Custom { message, .. } => {
                write!(f, "{message}")
            }
        }
    }
}

impl StdError for ResilienceError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Custom {
                source: Some(src), ..
            } => Some(src.as_ref() as &(dyn StdError + 'static)),
            _ => None,
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
