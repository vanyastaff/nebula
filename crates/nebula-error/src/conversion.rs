//! Error conversion utilities for Nebula

use super::error::NebulaError;
use std::fmt;

/// Trait for converting external errors to NebulaError
pub trait IntoNebulaError {
    /// Convert to NebulaError
    fn into_nebula_error(self) -> NebulaError;
}

/// Trait for converting external errors to NebulaError with context
pub trait IntoNebulaErrorWithContext {
    /// Convert to NebulaError with context
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError;
}

/// Extension trait for Results to add context
pub trait ResultContextExt<T, E> {
    /// Add context to a Result
    fn with_context(self, context: impl Into<String>) -> Result<T, NebulaError>;
}

impl<T, E> ResultContextExt<T, E> for Result<T, E>
where
    E: IntoNebulaError + IntoNebulaErrorWithContext,
{
    fn with_context(self, context: impl Into<String>) -> Result<T, NebulaError> {
        self.map_err(|e| e.into_nebula_error_with_context(context))
    }
}

// Implement conversions for common standard library errors
impl IntoNebulaError for std::io::Error {
    fn into_nebula_error(self) -> NebulaError {
        match self.kind() {
            std::io::ErrorKind::NotFound => NebulaError::not_found("File", "unknown"),
            std::io::ErrorKind::PermissionDenied => NebulaError::permission_denied("read", "file"),
            std::io::ErrorKind::TimedOut => {
                NebulaError::timeout("I/O operation", std::time::Duration::from_secs(30))
            },
            std::io::ErrorKind::ConnectionRefused => {
                NebulaError::service_unavailable("network", "connection refused", None)
            },
            std::io::ErrorKind::ConnectionReset => NebulaError::network("connection reset"),
            std::io::ErrorKind::BrokenPipe => NebulaError::network("broken pipe"),
            std::io::ErrorKind::WouldBlock => {
                NebulaError::timeout("I/O operation", std::time::Duration::from_millis(100))
            },
            _ => NebulaError::internal(format!("I/O error: {}", self)),
        }
    }
}

impl IntoNebulaErrorWithContext for std::io::Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

impl IntoNebulaError for std::fmt::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::internal("Formatting error")
    }
}

impl IntoNebulaErrorWithContext for std::fmt::Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for serialization errors
impl IntoNebulaError for serde_json::Error {
    fn into_nebula_error(self) -> NebulaError {
        match self.classify() {
            serde_json::error::Category::Io => NebulaError::internal("JSON I/O error"),
            serde_json::error::Category::Syntax => NebulaError::validation("Invalid JSON syntax"),
            serde_json::error::Category::Data => NebulaError::validation("Invalid JSON data"),
            serde_json::error::Category::Eof => {
                NebulaError::validation("Unexpected end of JSON input")
            },
        }
    }
}

impl IntoNebulaErrorWithContext for serde_json::Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

impl IntoNebulaError for bincode::Error {
    fn into_nebula_error(self) -> NebulaError {
        // Convert bincode error to string and create a validation error
        NebulaError::validation(format!("Bincode error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for bincode::Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for UUID errors
impl IntoNebulaError for uuid::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::validation(format!("UUID error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for uuid::Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for chrono errors
impl IntoNebulaError for chrono::ParseError {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::validation(format!("Date/time parsing error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for chrono::ParseError {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for anyhow errors
impl IntoNebulaError for anyhow::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::internal(format!("Anyhow error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for anyhow::Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for tokio errors
impl IntoNebulaError for tokio::time::error::Elapsed {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::timeout("operation", std::time::Duration::from_secs(30))
    }
}

impl IntoNebulaErrorWithContext for tokio::time::error::Elapsed {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for serde errors
impl IntoNebulaError for serde::de::value::Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::deserialization(format!("Deserialization error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for serde::de::value::Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for std::num errors
impl IntoNebulaError for std::num::ParseIntError {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::validation(format!("Integer parsing error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for std::num::ParseIntError {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

impl IntoNebulaError for std::num::ParseFloatError {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::validation(format!("Float parsing error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for std::num::ParseFloatError {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for std::str errors
impl IntoNebulaError for std::str::Utf8Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::validation(format!("UTF-8 error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for std::str::Utf8Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Implement conversions for std::string errors
impl IntoNebulaError for std::string::FromUtf8Error {
    fn into_nebula_error(self) -> NebulaError {
        NebulaError::validation(format!("UTF-8 conversion error: {}", self))
    }
}

impl IntoNebulaErrorWithContext for std::string::FromUtf8Error {
    fn into_nebula_error_with_context(self, context: impl Into<String>) -> NebulaError {
        self.into_nebula_error().with_context(super::context::ErrorContext::new(context))
    }
}

// Helper function to convert any error that implements Display
pub fn from_display_error<E: fmt::Display>(error: E) -> NebulaError {
    NebulaError::internal(format!("Error: {}", error))
}

// Helper function to convert any error that implements Display with context
pub fn from_display_error_with_context<E: fmt::Display>(
    error: E,
    context: impl Into<String>,
) -> NebulaError {
    from_display_error(error).with_context(super::context::ErrorContext::new(context))
}

// Helper function to convert any error that implements std::error::Error
pub fn from_std_error<E: std::error::Error>(error: E) -> NebulaError {
    NebulaError::internal(format!("Standard error: {}", error))
}

// Helper function to convert any error that implements std::error::Error with context
pub fn from_std_error_with_context<E: std::error::Error>(
    error: E,
    context: impl Into<String>,
) -> NebulaError {
    from_std_error(error).with_context(super::context::ErrorContext::new(context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_io_error_conversion() {
        let io_error = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let nebula_error = io_error.into_nebula_error();

        assert!(matches!(nebula_error.kind, super::super::error::ErrorKind::NotFound { .. }));
    }

    #[test]
    fn test_io_error_conversion_with_context() {
        let io_error = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
        let nebula_error = io_error.into_nebula_error_with_context("file operation");

        assert!(matches!(
            nebula_error.kind,
            super::super::error::ErrorKind::PermissionDenied { .. }
        ));
        assert!(nebula_error.context.is_some());
    }

    #[test]
    fn test_json_error_conversion() {
        let json_str = "invalid json";
        let json_error = serde_json::from_str::<serde_json::Value>(json_str).unwrap_err();
        let nebula_error = json_error.into_nebula_error();

        assert!(matches!(nebula_error.kind, super::super::error::ErrorKind::Validation { .. }));
    }

    #[test]
    fn test_result_context_extension() {
        let result: Result<(), io::Error> =
            Err(io::Error::new(io::ErrorKind::NotFound, "not found"));
        let result_with_context = result.with_context("file operation");

        assert!(result_with_context.is_err());
        let error = result_with_context.unwrap_err();
        assert!(error.context.is_some());
    }

    #[test]
    fn test_display_error_conversion() {
        let error = "custom error message";
        let nebula_error = from_display_error(error);

        assert!(matches!(nebula_error.kind, super::super::error::ErrorKind::Internal { .. }));
        assert!(nebula_error.message.contains("custom error message"));
    }

    #[test]
    fn test_std_error_conversion() {
        let error = io::Error::new(io::ErrorKind::Other, "other error");
        let nebula_error = from_std_error(error);

        assert!(matches!(nebula_error.kind, super::super::error::ErrorKind::Internal { .. }));
    }
}
