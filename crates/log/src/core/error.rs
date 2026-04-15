//! Error handling for nebula-log

use thiserror::Error;

/// Result type for logging operations
pub type LogResult<T> = Result<T, LogError>;

/// Error types for logging operations
#[derive(Error, Debug, nebula_error::Classify)]
#[non_exhaustive]
pub enum LogError {
    /// Configuration error occurred
    #[classify(category = "validation", code = "LOG:CONFIG")]
    #[error("Configuration error: {0}")]
    Config(String),
    /// Filter parsing failed
    #[classify(category = "validation", code = "LOG:FILTER")]
    #[error("Filter parsing error: {0}")]
    Filter(String),
    /// Configuration precedence resolution failed
    #[classify(category = "validation", code = "LOG:PRECEDENCE")]
    #[error("Precedence error: {0}")]
    Precedence(String),
    /// Policy parsing/validation failed
    #[classify(category = "validation", code = "LOG:POLICY")]
    #[error("Policy error: {0}")]
    Policy(String),
    /// Writer or I/O operation failed
    #[classify(category = "internal", code = "LOG:IO")]
    #[error("IO error: {0}")]
    Io(String),
    /// Telemetry setup failed
    #[classify(category = "external", code = "LOG:TELEMETRY")]
    #[error("Telemetry error: {0}")]
    Telemetry(String),
    /// Internal logging error
    #[classify(category = "internal", code = "LOG:INTERNAL")]
    #[error("Internal error: {0}")]
    Internal(String),
    /// Logger already initialized for this process
    ///
    /// Returned by [`init_with`](crate::init_with) / [`init`](crate::init) /
    /// [`auto_init`](crate::auto_init) when `tracing::dispatcher::has_been_set()`
    /// is already true. Callers that expect idempotent initialization can treat
    /// this variant as success.
    #[classify(category = "validation", code = "LOG:ALREADY_INITIALIZED")]
    #[error("Logger already initialized for this process")]
    AlreadyInitialized,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_initialized_display_is_stable() {
        let err = LogError::AlreadyInitialized;
        assert_eq!(
            err.to_string(),
            "Logger already initialized for this process"
        );
    }

    #[test]
    fn already_initialized_has_stable_classification() {
        use nebula_error::Classify;

        let err = LogError::AlreadyInitialized;
        // Lock in the error code string so downstream consumers can match on it.
        assert_eq!(err.code(), "LOG:ALREADY_INITIALIZED");
        // And distinct from the nearest neighbour we could have re-used instead.
        let internal = LogError::Internal(String::new());
        assert_ne!(err.code(), internal.code());
    }
}
