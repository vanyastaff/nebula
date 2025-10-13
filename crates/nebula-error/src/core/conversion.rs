//! Error conversion utilities for Nebula
//!
//! This module provides conversion utilities to transform external error types
//! into [`NebulaError`] instances with appropriate categorization and context.

use crate::core::context::ErrorContext;
use crate::core::{NebulaError, Result};
use crate::kinds::{ClientError, ErrorKind, ServerError, SystemError};
use std::time::Duration;

/// Trait for converting external errors to [`NebulaError`]
pub trait IntoNebulaError {
    /// Convert to [`NebulaError`]
    fn into_nebula_error(self) -> NebulaError;
}

/// Extension trait for `Result`s to add context (conversion variant)
///
/// Note: This provides the same interface as `ResultExt` but for conversion contexts.
/// Currently unused - the public `ResultExt` trait in result.rs is preferred.
/// Kept for potential future use with specific conversion scenarios.
#[allow(dead_code)]
trait ConversionResultExt<T, E> {
    /// Add context to a Result
    fn context(self, context: impl Into<String>) -> Result<T>;

    /// Add context with metadata
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> ErrorContext;
}

impl<T, E> ConversionResultExt<T, E> for std::result::Result<T, E>
where
    E: IntoNebulaError,
{
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|e| {
            e.into_nebula_error()
                .with_context(ErrorContext::new(context))
        })
    }

    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> ErrorContext,
    {
        self.map_err(|e| e.into_nebula_error().with_context(f()))
    }
}

// =============================================================================
// Standard Library Error Conversions
// =============================================================================

impl IntoNebulaError for std::io::Error {
    fn into_nebula_error(self) -> NebulaError {
        let kind = match self.kind() {
            std::io::ErrorKind::NotFound => {
                ErrorKind::Client(ClientError::not_found("File", "unknown"))
            }
            std::io::ErrorKind::PermissionDenied => {
                ErrorKind::Client(ClientError::permission_denied("read", "file"))
            }
            std::io::ErrorKind::TimedOut => ErrorKind::System(SystemError::timeout(
                "I/O operation",
                Duration::from_secs(30),
            )),
            std::io::ErrorKind::ConnectionRefused => {
                ErrorKind::System(SystemError::connection("unknown", "connection refused"))
            }
            std::io::ErrorKind::ConnectionReset => {
                ErrorKind::System(SystemError::network("connection reset"))
            }
            std::io::ErrorKind::BrokenPipe => {
                ErrorKind::System(SystemError::network("broken pipe"))
            }
            std::io::ErrorKind::WouldBlock => ErrorKind::System(SystemError::timeout(
                "I/O operation",
                Duration::from_millis(100),
            )),
            _ => ErrorKind::System(SystemError::file_system("I/O operation", self.to_string())),
        };
        NebulaError::new(kind)
    }
}

impl IntoNebulaError for std::fmt::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Server(ServerError::internal("Formatting error")))
    }
}

impl IntoNebulaError for std::num::ParseIntError {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::validation(format!(
            "Integer parsing error: {self}"
        ))))
    }
}

impl IntoNebulaError for std::num::ParseFloatError {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::validation(format!(
            "Float parsing error: {self}"
        ))))
    }
}

impl IntoNebulaError for std::str::Utf8Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::validation(format!(
            "UTF-8 error: {self}"
        ))))
    }
}

impl IntoNebulaError for std::string::FromUtf8Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::validation(format!(
            "UTF-8 conversion error: {self}"
        ))))
    }
}

// =============================================================================
// Third-party Crate Error Conversions
// =============================================================================

impl IntoNebulaError for serde_json::Error {
    fn into_nebula_error(self) -> NebulaError {
        let kind = match self.classify() {
            serde_json::error::Category::Io => {
                ErrorKind::System(SystemError::file_system("JSON I/O", self.to_string()))
            }
            serde_json::error::Category::Syntax => {
                ErrorKind::Client(ClientError::validation("Invalid JSON syntax"))
            }
            serde_json::error::Category::Data => {
                ErrorKind::Client(ClientError::validation("Invalid JSON data"))
            }
            serde_json::error::Category::Eof => {
                ErrorKind::Client(ClientError::validation("Unexpected end of JSON input"))
            }
        };
        NebulaError::new(kind)
    }
}

impl IntoNebulaError for bincode::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::deserialization(format!(
            "Bincode error: {self}"
        ))))
    }
}

impl IntoNebulaError for uuid::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::validation(format!(
            "UUID error: {self}"
        ))))
    }
}

impl IntoNebulaError for chrono::ParseError {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::validation(format!(
            "Date/time parsing error: {self}"
        ))))
    }
}

impl IntoNebulaError for anyhow::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Server(ServerError::internal(format!(
            "Anyhow error: {self}"
        ))))
    }
}

impl IntoNebulaError for tokio::time::error::Elapsed {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::System(SystemError::timeout(
            "operation",
            Duration::from_secs(30),
        )))
    }
}

impl IntoNebulaError for serde::de::value::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Client(ClientError::deserialization(format!(
            "Deserialization error: {self}"
        ))))
    }
}

// =============================================================================
// String Conversions
// =============================================================================

impl IntoNebulaError for &str {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Server(ServerError::internal(self.to_string())))
    }
}

impl IntoNebulaError for String {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::new(ErrorKind::Server(ServerError::internal(self)))
    }
}

// =============================================================================
// From Implementations for Into<NebulaError> compatibility
// =============================================================================

impl From<std::io::Error> for NebulaError {
    fn from(error: std::io::Error) -> Self {
        error.into_nebula_error()
    }
}

impl From<serde_json::Error> for NebulaError {
    fn from(error: serde_json::Error) -> Self {
        error.into_nebula_error()
    }
}

impl From<&str> for NebulaError {
    fn from(error: &str) -> Self {
        error.into_nebula_error()
    }
}

impl From<String> for NebulaError {
    fn from(error: String) -> Self {
        error.into_nebula_error()
    }
}

// Self-conversion for NebulaError
impl IntoNebulaError for NebulaError {
    fn into_nebula_error(self) -> NebulaError {
        self
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Convert any error that implements `Display` to [`NebulaError`]
pub fn from_display_error<E: std::fmt::Display>(error: E) -> NebulaError {
    NebulaError::new(ErrorKind::Server(ServerError::internal(format!(
        "Error: {error}"
    ))))
}

/// Convert any error that implements `Display` to [`NebulaError`] with context
pub fn from_display_error_with_context<E: std::fmt::Display>(
    error: E,
    context: impl Into<String>,
) -> NebulaError {
    from_display_error(error).with_context(ErrorContext::new(context))
}

/// Convert any error that implements `std::error::Error` to [`NebulaError`]
pub fn from_std_error<E: std::error::Error>(error: E) -> NebulaError {
    NebulaError::new(ErrorKind::Server(ServerError::internal(format!(
        "Standard error: {error}"
    ))))
}

/// Convert any error that implements [`std::error::Error`] to [`NebulaError`] with context
pub fn from_std_error_with_context<E: std::error::Error>(
    error: E,
    context: impl Into<String>,
) -> NebulaError {
    from_std_error(error).with_context(ErrorContext::new(context))
}

/// Smart error conversion that attempts to classify errors automatically
///
/// This function examines the error message and type to choose the most appropriate
/// NebulaError classification, providing better automatic error handling.
pub fn smart_convert<E: std::error::Error>(error: E) -> NebulaError {
    let error_str = error.to_string().to_lowercase();
    
    // Classification based on common error message patterns
    if error_str.contains("not found") || error_str.contains("missing") {
        NebulaError::not_found("resource", "unknown")
    } else if error_str.contains("permission") || error_str.contains("access denied") || error_str.contains("forbidden") {
        NebulaError::permission_denied("access", "resource")
    } else if error_str.contains("timeout") || error_str.contains("timed out") {
        NebulaError::timeout("operation", Duration::from_secs(30))
    } else if error_str.contains("connection") && (error_str.contains("refused") || error_str.contains("failed")) {
        NebulaError::network("connection error")
    } else if error_str.contains("invalid") || error_str.contains("parse") || error_str.contains("format") {
        NebulaError::validation(error.to_string())
    } else if error_str.contains("rate limit") || error_str.contains("too many") {
        NebulaError::rate_limit_exceeded(100, Duration::from_secs(60))
    } else {
        // Default to internal error for unknown cases
        NebulaError::internal(error.to_string())
    }
}

/// Macro for converting external errors with automatic context inference
///
/// This macro attempts to provide sensible context based on the call site
#[macro_export]
macro_rules! convert_error {
    ($error:expr) => {
        $crate::core::conversion::smart_convert($error)
    };
    ($error:expr, $context:expr) => {
        $crate::core::conversion::from_std_error_with_context($error, $context)
    };
}

/// Trait for enhanced error chaining
pub trait ErrorChain {
    /// Chain this error with additional context
    fn chain_with(self, msg: &str) -> NebulaError;
    
    /// Chain this error and mark as retryable
    fn chain_retryable(self) -> NebulaError;
    
    /// Chain this error and mark as non-retryable
    fn chain_terminal(self) -> NebulaError;
}

impl<E: std::error::Error> ErrorChain for E {
    fn chain_with(self, msg: &str) -> NebulaError {
        smart_convert(self).with_details(msg)
    }
    
    fn chain_retryable(self) -> NebulaError {
        smart_convert(self).with_retry_info(true, Some(Duration::from_secs(1)))
    }
    
    fn chain_terminal(self) -> NebulaError {
        smart_convert(self).with_retry_info(false, None)
    }
}

/// Optimized conversion for common error types using zero-cost static dispatch
pub trait OptimizedConvert {
    fn to_nebula_error(self) -> NebulaError;
}

impl OptimizedConvert for std::io::Error {
    #[inline]
    fn to_nebula_error(self) -> NebulaError {
        self.into_nebula_error()
    }
}

impl OptimizedConvert for serde_json::Error {
    #[inline]
    fn to_nebula_error(self) -> NebulaError {
        self.into_nebula_error()
    }
}

impl OptimizedConvert for &'static str {
    #[inline]
    fn to_nebula_error(self) -> NebulaError {
        NebulaError::new_static(
            ErrorKind::Server(ServerError::internal(self.to_string())),
            self
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_io_error_conversion() {
        let io_error = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let nebula_error = io_error.into_nebula_error();

        assert!(nebula_error.is_client_error());
        assert!(!nebula_error.is_retryable());
        assert_eq!(nebula_error.error_code(), "NOT_FOUND_ERROR");
    }

    #[test]
    fn test_timeout_error_conversion() {
        let io_error = io::Error::new(io::ErrorKind::TimedOut, "operation timed out");
        let nebula_error = io_error.into_nebula_error();

        assert!(nebula_error.is_system_error());
        assert!(nebula_error.is_retryable());
        assert_eq!(nebula_error.error_code(), "TIMEOUT_ERROR");
    }

    #[test]
    fn test_json_error_conversion() {
        let json_str = "invalid json";
        let json_error = serde_json::from_str::<serde_json::Value>(json_str).unwrap_err();
        let nebula_error = json_error.into_nebula_error();

        assert!(nebula_error.is_client_error());
        assert!(!nebula_error.is_retryable());
        assert_eq!(nebula_error.error_code(), "VALIDATION_ERROR");
    }

    #[test]
    fn test_result_context_extension() {
        let result: std::result::Result<(), io::Error> =
            Err(io::Error::new(io::ErrorKind::NotFound, "not found"));
        let result_with_context = result.context("file operation");

        assert!(result_with_context.is_err());
        let error = result_with_context.unwrap_err();
        assert!(error.context.is_some());
        assert_eq!(error.context().unwrap().description, "file operation");
    }

    #[test]
    fn test_display_error_conversion() {
        let error = "custom error message";
        let nebula_error = from_display_error(error);

        assert!(nebula_error.is_server_error());
        assert_eq!(nebula_error.error_code(), "INTERNAL_ERROR");
        assert!(nebula_error.user_message().contains("custom error message"));
    }

    #[test]
    fn test_string_error_conversion() {
        let error_msg = "Something went wrong".to_string();
        let nebula_error = error_msg.into_nebula_error();

        assert!(nebula_error.is_server_error());
        assert_eq!(nebula_error.error_code(), "INTERNAL_ERROR");
        assert!(nebula_error.user_message().contains("Something went wrong"));
    }
}
