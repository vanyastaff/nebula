//! Error handling for nebula-log

use thiserror::Error;

/// Result type for logging operations
pub type LogResult<T> = Result<T, LogError>;

/// Error types for logging operations
#[derive(Error, Debug)]
pub enum LogError {
    /// Configuration error occurred
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    /// Filter parsing failed
    #[error("Filter parsing error: {0}")]
    FilterParsingError(String),
    /// Writer initialization failed
    #[error("Writer initialization error: {0}")]
    WriterInitializationError(String),
    /// Telemetry setup failed
    #[error("Telemetry setup error: {0}")]
    TelemetrySetupError(String),
    /// Formatting error occurred
    #[error("Format error: {0}")]
    FormatError(String),
    /// Log rotation failed
    #[error("Log rotation error: {0}")]
    LogRotationError(String),
    /// Internal logging error
    #[error("Internal error: {0}")]
    InternalError(String),
    /// I/O operation failed
    #[error("IO error: {0}")]
    IoError(String),
    /// Invalid input provided
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    /// Invalid argument provided
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    /// Invalid state encountered
    #[error("Invalid state: {0}")]
    InvalidState(String),
    /// Invalid operation attempted
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),
    /// Invalid configuration provided
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
    /// Invalid filter specified
    #[error("Invalid filter: {0}")]
    InvalidFilter(String),
    /// Invalid writer specified
    #[error("Invalid writer: {0}")]
    InvalidWriter(String),
}

impl LogError {
    /// Create a configuration error
    pub fn configuration_error(message: impl Into<String>) -> Self {
        Self::ConfigurationError(message.into())
    }

    /// Create a filter parsing error
    pub fn filter_parsing_error(message: impl Into<String>) -> Self {
        Self::FilterParsingError(message.into())
    }

    /// Create a writer initialization error
    pub fn writer_initialization_error(message: impl Into<String>) -> Self {
        Self::WriterInitializationError(message.into())
    }

    /// Create a telemetry setup error
    pub fn telemetry_setup_error(message: impl Into<String>) -> Self {
        Self::TelemetrySetupError(message.into())
    }

    /// Create a format error
    pub fn format_error(message: impl Into<String>) -> Self {
        Self::FormatError(message.into())
    }

    /// Create a log rotation error
    pub fn log_rotation_error(message: impl Into<String>) -> Self {
        Self::LogRotationError(message.into())
    }

    /// Create an internal error
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::InternalError(message.into())
    }

    /// Create an I/O error
    pub fn io_error(message: impl Into<String>) -> Self {
        Self::IoError(message.into())
    }
}
