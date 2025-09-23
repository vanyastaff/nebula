//! Error handling for nebula-value
//!
//! This module provides unified error handling using NebulaError from the nebula-error crate.
//! All value operations return `ValueResult<T>` which is an alias for `Result<T, NebulaError>`.

use nebula_error::{NebulaError, Result as NebulaResult};

/// Type alias for Result with NebulaError for value operations
pub type ValueResult<T> = NebulaResult<T>;

// ==================== Value-specific NebulaError Extensions ====================

/// Extension trait for creating value-specific NebulaErrors
pub trait ValueErrorExt {
    /// Create a value type mismatch error
    fn value_type_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self;

    /// Create a value conversion error
    fn value_conversion_error(from: impl Into<String>, to: impl Into<String>) -> Self;

    /// Create a value index out of bounds error
    fn value_index_out_of_bounds(index: usize, length: usize) -> Self;

    /// Create a value key not found error
    fn value_key_not_found(key: impl Into<String>) -> Self;

    /// Create a value path not found error
    fn value_path_not_found(path: impl Into<String>) -> Self;

    /// Create a value parse error
    fn value_parse_error(format_type: impl Into<String>, input: impl Into<String>) -> Self;

    /// Create a value operation not supported error
    fn value_operation_not_supported(operation: impl Into<String>, value_type: impl Into<String>) -> Self;
}

impl ValueErrorExt for NebulaError {
    /// Create a value type mismatch error
    fn value_type_mismatch(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self::validation(format!("Type mismatch: expected {}, got {}", expected.into(), actual.into()))
    }

    /// Create a value conversion error
    fn value_conversion_error(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::validation(format!("Cannot convert from {} to {}", from.into(), to.into()))
    }

    /// Create a value index out of bounds error
    fn value_index_out_of_bounds(index: usize, length: usize) -> Self {
        Self::not_found("array_index", index.to_string())
            .with_details(format!("Index {} out of bounds (length: {})", index, length))
    }

    /// Create a value key not found error
    fn value_key_not_found(key: impl Into<String>) -> Self {
        Self::not_found("object_key", key)
    }

    /// Create a value path not found error
    fn value_path_not_found(path: impl Into<String>) -> Self {
        Self::not_found("path", path)
    }

    /// Create a value parse error
    fn value_parse_error(format_type: impl Into<String>, input: impl Into<String>) -> Self {
        Self::validation(format!("Invalid {} format: {}", format_type.into(), input.into()))
    }

    /// Create a value operation not supported error
    fn value_operation_not_supported(operation: impl Into<String>, value_type: impl Into<String>) -> Self {
        Self::validation(format!("Operation '{}' not supported for {}", operation.into(), value_type.into()))
    }
}

// ==================== Result helpers ====================

/// Extension trait for Result types (value-specific)
pub trait ValueResultExt<T> {
    /// Convert to NebulaError with custom message
    fn or_error<S: Into<String>>(self, msg: S) -> ValueResult<T>;

    /// Add context to error
    fn with_context<S: Into<String>, F>(self, f: F) -> ValueResult<T>
    where
        F: FnOnce() -> S;
}

impl<T, E> ValueResultExt<T> for Result<T, E>
where
    E: std::error::Error,
{
    fn or_error<S: Into<String>>(self, msg: S) -> ValueResult<T> {
        self.map_err(|_| NebulaError::internal(msg))
    }

    fn with_context<S: Into<String>, F>(self, f: F) -> ValueResult<T>
    where
        F: FnOnce() -> S,
    {
        self.map_err(|e| NebulaError::internal(format!("{}: {}", f().into(), e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_error_ext() {
        let error = NebulaError::value_type_mismatch("string", "integer");
        assert!(error.is_client_error());
        assert_eq!(error.error_code(), "VALIDATION_ERROR");

        let error = NebulaError::value_index_out_of_bounds(5, 3);
        assert!(error.is_client_error());
        assert_eq!(error.error_code(), "NOT_FOUND_ERROR");

        let error = NebulaError::value_operation_not_supported("add", "boolean");
        assert!(error.is_client_error());
        assert_eq!(error.error_code(), "VALIDATION_ERROR");
    }

    #[test]
    fn test_value_result_ext() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let value_result = result.or_error("Custom error message");

        assert!(value_result.is_err());
        let error = value_result.unwrap_err();
        assert!(error.is_server_error());
        assert!(error.user_message().contains("Custom error message"));
    }

    #[test]
    fn test_value_result_with_context() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(std::io::ErrorKind::NotFound, "test"));
        let value_result = result.with_context(|| "Processing value");

        assert!(value_result.is_err());
        let error = value_result.unwrap_err();
        assert!(error.user_message().contains("Processing value"));
    }
}