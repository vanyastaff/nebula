//! Error handling for nebula-log

use thiserror::Error;

/// Result type for logging operations
pub type LogResult<T> = Result<T, LogError>;

/// Error types for logging operations
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum LogError {
    /// Configuration error occurred
    #[error("Configuration error: {0}")]
    Config(String),
    /// Filter parsing failed
    #[error("Filter parsing error: {0}")]
    Filter(String),
    /// Writer or I/O operation failed
    #[error("IO error: {0}")]
    Io(String),
    /// Telemetry setup failed
    #[error("Telemetry error: {0}")]
    Telemetry(String),
    /// Internal logging error
    #[error("Internal error: {0}")]
    Internal(String),
}
