//! Result type and extension traits for logging operations

use crate::core::error::{LogError, LogResult};
use nebula_error::{ErrorContext, NebulaError};

/// Extension trait for Result types (log-specific)
pub trait LogResultExt<T> {
    /// Convert to [`LogResult`] with custom log error message
    ///
    /// # Errors
    ///
    /// Returns [`NebulaError::Internal`] with the provided message
    fn or_log_error<S: Into<String>>(self, msg: S) -> LogResult<T>;

    /// Add logging context to error
    ///
    /// # Errors
    ///
    /// Returns [`NebulaError::Internal`] with context prepended to error message
    fn with_log_context<S: Into<String>, F>(self, f: F) -> LogResult<T>
    where
        F: FnOnce() -> S;

    /// Add component context for logging operations
    ///
    /// # Errors
    ///
    /// Returns error with component metadata attached via [`ErrorContext`]
    fn with_component(self, component: impl Into<String>) -> LogResult<T>;

    /// Add operation context for logging operations
    ///
    /// # Errors
    ///
    /// Returns error with operation metadata attached via [`ErrorContext`]
    fn with_operation(self, operation: impl Into<String>) -> LogResult<T>;
}

impl<T, E> LogResultExt<T> for Result<T, E>
where
    E: std::error::Error,
{
    fn or_log_error<S: Into<String>>(self, msg: S) -> LogResult<T> {
        self.map_err(|_| NebulaError::internal(msg))
    }

    fn with_log_context<S: Into<String>, F>(self, f: F) -> LogResult<T>
    where
        F: FnOnce() -> S,
    {
        self.map_err(|e| {
            let ctx = f().into();
            NebulaError::internal(format!("{ctx}: {e}"))
        })
    }

    fn with_component(self, component: impl Into<String>) -> LogResult<T> {
        self.map_err(|e| {
            NebulaError::internal(format!("{e}")).with_context(
                ErrorContext::new("Logging operation failed").with_component(component),
            )
        })
    }

    fn with_operation(self, operation: impl Into<String>) -> LogResult<T> {
        self.map_err(|e| {
            NebulaError::internal(format!("{e}")).with_context(
                ErrorContext::new("Logging operation failed").with_operation(operation),
            )
        })
    }
}

/// Extension trait specifically for IO Result types
pub trait LogIoResultExt {
    /// Convert IO error to [`LogResult`] with custom message
    ///
    /// # Errors
    ///
    /// Returns log writer error with custom message prepended
    fn or_log_error<S: Into<String>>(self, msg: S) -> LogResult<()>;

    /// Add logging context to IO error
    ///
    /// # Errors
    ///
    /// Returns log writer error with context prepended
    fn with_log_context<S: Into<String>, F>(self, f: F) -> LogResult<()>
    where
        F: FnOnce() -> S;

    /// Add component context for IO operations
    ///
    /// # Errors
    ///
    /// Returns error with component metadata attached
    fn with_component(self, component: impl Into<String>) -> LogResult<()>;

    /// Add operation context for IO operations
    ///
    /// # Errors
    ///
    /// Returns error with operation metadata attached
    fn with_operation(self, operation: impl Into<String>) -> LogResult<()>;
}

// Specific implementations for common error types
impl LogIoResultExt for Result<(), std::io::Error> {
    fn or_log_error<S: Into<String>>(self, msg: S) -> LogResult<()> {
        self.map_err(|e| {
            let msg = msg.into();
            NebulaError::log_writer_error("io", format!("{msg}: {e}"))
        })
    }

    fn with_log_context<S: Into<String>, F>(self, f: F) -> LogResult<()>
    where
        F: FnOnce() -> S,
    {
        self.map_err(|e| {
            let ctx = f().into();
            NebulaError::log_writer_error("io", format!("{ctx}: {e}"))
        })
    }

    fn with_component(self, component: impl Into<String>) -> LogResult<()> {
        self.map_err(|e| {
            NebulaError::log_writer_error("io", format!("{e}"))
                .with_context(ErrorContext::new("IO operation failed").with_component(component))
        })
    }

    fn with_operation(self, operation: impl Into<String>) -> LogResult<()> {
        self.map_err(|e| {
            NebulaError::log_writer_error("io", format!("{e}"))
                .with_context(ErrorContext::new("IO operation failed").with_operation(operation))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_log_io_result_ext() {
        let result: Result<(), io::Error> = Err(io::Error::new(io::ErrorKind::NotFound, "test"));
        let log_result = LogIoResultExt::or_log_error(result, "File operation failed");

        assert!(log_result.is_err());
        let error = log_result.unwrap_err();
        assert!(error.is_server_error());
        assert!(error.user_message().contains("File operation failed"));
    }

    #[test]
    fn test_with_component() {
        let result: Result<(), io::Error> = Err(io::Error::new(io::ErrorKind::NotFound, "test"));
        let log_result = LogIoResultExt::with_component(result, "file-appender");

        assert!(log_result.is_err());
        let error = log_result.unwrap_err();
        assert!(error.context().is_some());
        assert_eq!(
            error.context().unwrap().component(),
            &Some("file-appender".to_string())
        );
    }
}
