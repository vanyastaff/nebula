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
    /// Configuration precedence resolution failed
    #[error("Precedence error: {0}")]
    Precedence(String),
    /// Policy parsing/validation failed
    #[error("Policy error: {0}")]
    Policy(String),
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

impl nebula_error::Classify for LogError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Config(_) | Self::Filter(_) | Self::Precedence(_) | Self::Policy(_) => {
                nebula_error::ErrorCategory::Validation
            }
            Self::Io(_) => nebula_error::ErrorCategory::Internal,
            Self::Telemetry(_) => nebula_error::ErrorCategory::External,
            Self::Internal(_) => nebula_error::ErrorCategory::Internal,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::Config(_) => "LOG:CONFIG",
            Self::Filter(_) => "LOG:FILTER",
            Self::Precedence(_) => "LOG:PRECEDENCE",
            Self::Policy(_) => "LOG:POLICY",
            Self::Io(_) => "LOG:IO",
            Self::Telemetry(_) => "LOG:TELEMETRY",
            Self::Internal(_) => "LOG:INTERNAL",
        })
    }
}
