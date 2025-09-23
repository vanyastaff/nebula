//! Common traits for error handling

use crate::core::error::NebulaError;
use crate::core::context::ErrorContext;

/// Trait for types that can be converted into NebulaError
pub trait IntoNebulaError {
    /// Convert this error into a NebulaError
    fn into_nebula_error(self) -> NebulaError;
}

/// Trait for converting errors with additional context
pub trait IntoNebulaErrorWithContext {
    /// Convert this error into a NebulaError with context
    fn into_nebula_error_with_context(self, context: ErrorContext) -> NebulaError;
}

/// Default implementation for any type that implements IntoNebulaError
impl<T: IntoNebulaError> IntoNebulaErrorWithContext for T {
    fn into_nebula_error_with_context(self, context: ErrorContext) -> NebulaError {
        self.into_nebula_error().with_context(context)
    }
}

/// Trait for error types that can determine if they are retryable
pub trait RetryableError {
    /// Check if this error should be retried
    fn is_retryable(&self) -> bool;

    /// Get suggested retry delay
    fn retry_delay(&self) -> Option<std::time::Duration> {
        None
    }
}

/// Trait for error classification
pub trait ErrorClassification {
    /// Check if this is a client error (user error, invalid input, etc.)
    fn is_client_error(&self) -> bool;

    /// Check if this is a server error (internal errors, service issues, etc.)
    fn is_server_error(&self) -> bool;

    /// Check if this is a system error (infrastructure, network, etc.)
    fn is_system_error(&self) -> bool;
}

/// Trait for getting error codes
pub trait ErrorCode {
    /// Get the error code for programmatic handling
    fn error_code(&self) -> &str;

    /// Get the error category
    fn error_category(&self) -> &str {
        "UNKNOWN"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::conversion::IntoNebulaError;

    #[test]
    fn test_into_nebula_error_with_context() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let context = ErrorContext::new("Reading configuration file");

        let nebula_error = io_error.into_nebula_error().with_context(context);

        assert!(nebula_error.context.is_some());
        assert_eq!(nebula_error.context().unwrap().description, "Reading configuration file");
    }

    #[test]
    fn test_error_classification() {
        let validation_error = NebulaError::validation("Invalid input");

        assert!(validation_error.is_client_error());
        assert!(!validation_error.is_server_error());
        assert!(!validation_error.is_system_error());
    }
}