//! Error types for resilience operations

use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

// Import NebulaError for integration
use nebula_error::NebulaError;

/// Core resilience errors
#[derive(Debug)]
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
                write!(f, "Operation timed out after {:?}", duration)?;
                if let Some(ctx) = context {
                    write!(f, " - {}", ctx)?;
                }
                Ok(())
            }
            Self::CircuitBreakerOpen { state, retry_after } => {
                write!(f, "Circuit breaker is {} ", state)?;
                if let Some(duration) = retry_after {
                    write!(f, "(retry after {:?})", duration)?;
                }
                Ok(())
            }
            Self::BulkheadFull {
                max_concurrency,
                queued,
            } => {
                write!(
                    f,
                    "Bulkhead full: max={}, queued={}",
                    max_concurrency, queued
                )
            }
            Self::RateLimitExceeded {
                limit,
                current,
                retry_after,
            } => {
                write!(
                    f,
                    "Rate limit exceeded: limit={}/s, current={}/s",
                    limit, current
                )?;
                if let Some(duration) = retry_after {
                    write!(f, " (retry after {:?})", duration)?;
                }
                Ok(())
            }
            Self::RetryLimitExceeded {
                attempts,
                last_error,
            } => {
                write!(f, "Retry limit exceeded after {} attempts", attempts)?;
                if let Some(err) = last_error {
                    write!(f, " - last error: {}", err)?;
                }
                Ok(())
            }
            Self::FallbackFailed { reason, .. } => {
                write!(f, "Fallback failed: {}", reason)
            }
            Self::Cancelled { reason } => {
                write!(f, "Operation cancelled")?;
                if let Some(r) = reason {
                    write!(f, ": {}", r)?;
                }
                Ok(())
            }
            Self::InvalidConfig { message } => {
                write!(f, "Invalid configuration: {}", message)
            }
            Self::Custom { message, .. } => {
                write!(f, "{}", message)
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
    /// Create a timeout error
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
    pub fn bulkhead_full(max_concurrency: usize) -> Self {
        Self::BulkheadFull {
            max_concurrency,
            queued: 0, // Default
        }
    }

    /// Create a retry limit exceeded error with cause
    pub fn retry_limit_exceeded_with_cause(attempts: usize, last_error: Option<Box<Self>>) -> Self {
        Self::RetryLimitExceeded {
            attempts,
            last_error,
        }
    }

    /// Classify the error for decision making
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
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.classify(),
            ErrorClass::Transient | ErrorClass::ResourceExhaustion
        )
    }

    /// Check if the error is terminal
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.classify(),
            ErrorClass::Permanent | ErrorClass::Configuration
        )
    }

    /// Get retry delay hint if available
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
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// ==================== NebulaError Integration ====================

impl From<ResilienceError> for NebulaError {
    fn from(err: ResilienceError) -> Self {
        match err {
            ResilienceError::Timeout { duration, context } => {
                let msg = match context {
                    Some(ctx) => format!("Operation timed out after {:?}: {}", duration, ctx),
                    None => format!("Operation timed out after {:?}", duration),
                };
                NebulaError::timeout("resilience-operation", duration).with_details(msg)
            }
            ResilienceError::CircuitBreakerOpen { state, retry_after } => {
                let msg = match retry_after {
                    Some(duration) => {
                        format!("Circuit breaker is {} (retry after {:?})", state, duration)
                    }
                    None => format!("Circuit breaker is {}", state),
                };
                NebulaError::service_unavailable("circuit-breaker", msg)
            }
            ResilienceError::BulkheadFull {
                max_concurrency,
                queued,
            } => NebulaError::new(nebula_error::ErrorKind::System(
                nebula_error::kinds::SystemError::resource_exhausted(format!(
                    "Bulkhead full: max={}, queued={}",
                    max_concurrency, queued
                )),
            )),
            ResilienceError::RateLimitExceeded {
                limit,
                current: _,
                retry_after: _,
            } => {
                // Convert f64 limit to u32 for NebulaError API
                let limit_u32 = limit as u32;
                let period = Duration::from_secs(1); // Assume per-second limit
                NebulaError::rate_limit_exceeded(limit_u32, period)
            }
            ResilienceError::RetryLimitExceeded {
                attempts,
                last_error,
            } => {
                let msg = match last_error {
                    Some(err) => {
                        format!("Retry limit exceeded after {} attempts: {}", attempts, err)
                    }
                    None => format!("Retry limit exceeded after {} attempts", attempts),
                };
                NebulaError::internal(msg)
            }
            ResilienceError::FallbackFailed {
                reason,
                original_error,
            } => {
                let msg = match original_error {
                    Some(err) => format!("Fallback failed: {} (original: {})", reason, err),
                    None => format!("Fallback failed: {}", reason),
                };
                NebulaError::internal(msg)
            }
            ResilienceError::Cancelled { reason } => {
                let msg = match reason {
                    Some(r) => r.clone(),
                    None => "Operation cancelled".to_string(),
                };
                NebulaError::execution_cancelled(msg)
            }
            ResilienceError::InvalidConfig { message } => {
                NebulaError::validation(format!("Invalid resilience configuration: {}", message))
            }
            ResilienceError::Custom {
                message,
                retryable,
                source: _,
            } => {
                if retryable {
                    NebulaError::service_unavailable("resilience-custom", message)
                } else {
                    NebulaError::internal(message)
                }
            }
        }
    }
}

impl From<NebulaError> for ResilienceError {
    fn from(err: NebulaError) -> Self {
        // Classify NebulaError into appropriate ResilienceError
        // Since specific is_* methods don't exist, we classify by error code or kind
        let code = err.error_code();

        if code.contains("timeout") {
            ResilienceError::Timeout {
                duration: err.retry_after().unwrap_or(Duration::from_secs(30)),
                context: Some(err.user_message().to_string()),
            }
        } else if code.contains("rate_limit") {
            ResilienceError::RateLimitExceeded {
                retry_after: err.retry_after(),
                limit: 100.0,   // Default limit
                current: 150.0, // Assumed over limit
            }
        } else if err.is_client_error() {
            ResilienceError::InvalidConfig {
                message: err.user_message().to_string(),
            }
        } else if code.contains("cancel") {
            ResilienceError::Cancelled {
                reason: Some(err.user_message().to_string()),
            }
        } else {
            // Map other errors to Custom
            ResilienceError::Custom {
                message: err.user_message().to_string(),
                retryable: err.is_retryable(),
                source: None,
            }
        }
    }
}
