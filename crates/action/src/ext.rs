//! Extension traits for ergonomic error conversion in actions.
//!
//! Provides `.retryable()?` and `.fatal()?` on any `Result<T, E>`
//! eliminating verbose `.map_err(...)` chains.
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_action::prelude::*;
//!
//! let response = client.get(url).await.retryable()?;
//! let data: MyData = response.json().await.fatal()?;
//! ```

use crate::error::{ActionError, ErrorCode};

/// Extension trait for converting `Result<T, E>` into `Result<T, ActionError>`.
///
/// Provides ergonomic `.retryable()?` and `.fatal()?` conversions that
/// eliminate verbose `.map_err(|e| ActionError::retryable(...))` chains.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_action::prelude::*;
///
/// fn fetch_data() -> Result<String, ActionError> {
///     let value: i32 = "42".parse().fatal()?;
///     Ok(format!("got {value}"))
/// }
/// ```
pub trait ActionResultExt<T> {
    /// Convert error to retryable [`ActionError`] (transient — engine may retry).
    ///
    /// Use for network errors, timeouts, and other transient failures where
    /// retrying the same operation may succeed.
    fn retryable(self) -> Result<T, ActionError>;

    /// Convert error to fatal [`ActionError`] (permanent — never retry).
    ///
    /// Use for validation errors, schema mismatches, and other permanent failures
    /// where retrying would produce the same error.
    fn fatal(self) -> Result<T, ActionError>;

    /// Convert error to retryable [`ActionError`] with a specific [`ErrorCode`].
    ///
    /// The error code enables the engine to apply smarter retry strategies
    /// (e.g., refresh credentials on [`ErrorCode::AuthExpired`]).
    fn retryable_with_code(self, code: ErrorCode) -> Result<T, ActionError>;

    /// Convert error to fatal [`ActionError`] with a specific [`ErrorCode`].
    ///
    /// The error code provides machine-readable classification for
    /// error reporting and monitoring.
    fn fatal_with_code(self, code: ErrorCode) -> Result<T, ActionError>;
}

impl<T, E> ActionResultExt<T> for Result<T, E>
where
    E: std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
{
    fn retryable(self) -> Result<T, ActionError> {
        self.map_err(ActionError::retryable)
    }

    fn fatal(self) -> Result<T, ActionError> {
        self.map_err(ActionError::fatal)
    }

    fn retryable_with_code(self, code: ErrorCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::retryable_with_code(e, code))
    }

    fn fatal_with_code(self, code: ErrorCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::fatal_with_code(e, code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_converts_io_error() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        let err = result.retryable().unwrap_err();
        assert!(err.is_retryable());
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn fatal_converts_io_error() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "corrupt",
        ));
        let err = result.fatal().unwrap_err();
        assert!(err.is_fatal());
    }

    #[test]
    fn retryable_with_code_sets_code() {
        let result: Result<i32, &str> = Err("rate limited");
        let err = result
            .retryable_with_code(ErrorCode::RateLimited)
            .unwrap_err();
        assert_eq!(err.error_code(), Some(&ErrorCode::RateLimited));
        assert!(err.is_retryable());
    }

    #[test]
    fn fatal_with_code_sets_code() {
        let result: Result<i32, &str> = Err("expired");
        let err = result.fatal_with_code(ErrorCode::AuthExpired).unwrap_err();
        assert_eq!(err.error_code(), Some(&ErrorCode::AuthExpired));
        assert!(err.is_fatal());
    }

    #[test]
    fn ok_passes_through() {
        let result: Result<i32, std::io::Error> = Ok(42);
        assert_eq!(result.retryable().unwrap(), 42);
    }

    #[test]
    fn chaining_in_function() {
        fn do_work() -> Result<String, ActionError> {
            let value: i32 = "42".parse().fatal()?;
            Ok(format!("result: {value}"))
        }
        assert_eq!(do_work().unwrap(), "result: 42");
    }

    #[test]
    fn string_error_retryable() {
        let result: Result<(), String> = Err("something failed".to_string());
        let err = result.retryable().unwrap_err();
        assert!(err.is_retryable());
        assert!(err.to_string().contains("something failed"));
    }
}
