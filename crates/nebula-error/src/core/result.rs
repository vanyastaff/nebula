//! Result type and extension traits

use crate::core::context::ErrorContext;
use crate::core::error::NebulaError;

/// Result type for Nebula operations
pub type Result<T> = std::result::Result<T, NebulaError>;

/// Extension trait for adding context to Results
pub trait ResultExt<T> {
    /// Add context to a Result
    fn context(self, context: impl Into<String>) -> Result<T>;

    /// Add context with metadata
    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> ErrorContext;

    /// Map the error type while preserving the success value
    fn map_nebula_err<F>(self, f: F) -> Result<T>
    where
        F: FnOnce(NebulaError) -> NebulaError;
}

impl<T, E> ResultExt<T> for std::result::Result<T, E>
where
    E: Into<NebulaError>,
{
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|e| {
            let mut nebula_error = e.into();
            nebula_error = nebula_error.with_context(ErrorContext::new(context));
            nebula_error
        })
    }

    fn with_context<F>(self, f: F) -> Result<T>
    where
        F: FnOnce() -> ErrorContext,
    {
        self.map_err(|e| {
            let mut nebula_error = e.into();
            nebula_error = nebula_error.with_context(f());
            nebula_error
        })
    }

    fn map_nebula_err<F>(self, f: F) -> Result<T>
    where
        F: FnOnce(NebulaError) -> NebulaError,
    {
        self.map_err(|e| f(e.into()))
    }
}

/// Extension trait specifically for NebulaError Results
pub trait NebulaResultExt<T> {
    /// Add details to the error if it fails
    fn with_details(self, details: impl Into<String>) -> Result<T>;

    /// Mark the error as retryable/non-retryable
    fn with_retryable(self, retryable: bool) -> Result<T>;

    /// Add retry delay information
    fn with_retry_after(self, retry_after: std::time::Duration) -> Result<T>;
}

impl<T> NebulaResultExt<T> for Result<T> {
    fn with_details(self, details: impl Into<String>) -> Result<T> {
        self.map_err(|e| e.with_details(details))
    }

    fn with_retryable(self, retryable: bool) -> Result<T> {
        self.map_err(|mut e| {
            e.retryable = retryable;
            e
        })
    }

    fn with_retry_after(self, retry_after: std::time::Duration) -> Result<T> {
        self.map_err(|mut e| {
            e.retry_after = Some(retry_after);
            e
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_extension() {
        let result: std::result::Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test error",
        ));
        let nebula_result = result.context("Operation failed");

        assert!(nebula_result.is_err());
        let error = nebula_result.unwrap_err();
        assert!(error.context.is_some());
        assert_eq!(error.context().unwrap().description, "Operation failed");
    }

    #[test]
    fn test_nebula_result_extensions() {
        let error = NebulaError::internal("Test error");
        let result: Result<()> = Err(error);

        let result_with_details = result
            .with_details("Additional debugging information")
            .with_retryable(true)
            .with_retry_after(std::time::Duration::from_secs(5));

        assert!(result_with_details.is_err());
        let error = result_with_details.unwrap_err();
        assert_eq!(error.details(), Some("Additional debugging information"));
        assert!(error.is_retryable());
        assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(5)));
    }

    #[test]
    fn test_context_with_metadata() {
        let result: std::result::Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "test error",
        ));
        let nebula_result = result.with_context(|| {
            ErrorContext::new("Operation failed")
                .with_component("user-service")
                .with_operation("create_user")
        });

        assert!(nebula_result.is_err());
        let error = nebula_result.unwrap_err();
        let context = error.context().unwrap();
        assert_eq!(context.description, "Operation failed");
        assert_eq!(context.component(), Some("user-service"));
        assert_eq!(context.operation(), Some("create_user"));
    }
}
