//! Error handling for nebula-log
//!
//! This module provides unified error handling using NebulaError from the nebula-error crate.
//! All logging operations return `LogResult<T>` which is an alias for `Result<T, NebulaError>`.

use nebula_error::{NebulaError, Result as NebulaResult};

/// Type alias for Result with NebulaError for logging operations
pub type LogResult<T> = NebulaResult<T>;

// ==================== Log-specific NebulaError Extensions ====================

/// Extension trait for creating log-specific NebulaErrors
pub trait LogError {
    /// Create a configuration error
    fn log_config_error(message: impl Into<String>) -> Self;

    /// Create a filter parsing error
    fn log_filter_error(filter: impl Into<String>, reason: impl Into<String>) -> Self;

    /// Create a writer initialization error
    fn log_writer_error(writer: impl Into<String>, reason: impl Into<String>) -> Self;

    /// Create a telemetry setup error
    fn log_telemetry_error(service: impl Into<String>, reason: impl Into<String>) -> Self;

    /// Create a log format error
    fn log_format_error(format: impl Into<String>, reason: impl Into<String>) -> Self;

    /// Create a log rotation error
    fn log_rotation_error(reason: impl Into<String>) -> Self;
}

impl LogError for NebulaError {
    /// Create a configuration error
    fn log_config_error(message: impl Into<String>) -> Self {
        Self::validation(format!("Configuration error: {}", message.into()))
    }

    /// Create a filter parsing error
    fn log_filter_error(filter: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::validation(format!("Invalid filter '{}': {}", filter.into(), reason.into()))
    }

    /// Create a writer initialization error
    fn log_writer_error(writer: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::internal(format!("Writer '{}' error: {}", writer.into(), reason.into()))
    }

    /// Create a telemetry setup error
    fn log_telemetry_error(service: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::internal(format!("Telemetry service '{}' error: {}", service.into(), reason.into()))
    }

    /// Create a log format error
    fn log_format_error(format: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::validation(format!("Format '{}' error: {}", format.into(), reason.into()))
    }

    /// Create a log rotation error
    fn log_rotation_error(reason: impl Into<String>) -> Self {
        Self::internal(format!("Log rotation error: {}", reason.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_error_ext() {
        let error = NebulaError::log_config_error("Invalid log level");
        assert!(error.is_client_error());
        assert_eq!(error.error_code(), "VALIDATION_ERROR");

        let error = NebulaError::log_filter_error("debug", "syntax error");
        assert!(error.is_client_error());
        assert!(error.user_message().contains("Invalid filter 'debug'"));

        let error = NebulaError::log_writer_error("file", "permission denied");
        assert!(error.is_server_error());
        assert!(error.user_message().contains("Writer 'file' error"));
    }
}