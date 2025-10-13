//! Convenient error macros for common patterns
//!
//! This module provides macros that make error creation more ergonomic
//! and less error-prone while maintaining zero-cost abstractions.

/// Create a validation error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::validation_error;
///
/// let field_name = "email";
/// let value = "invalid-email";
/// let error = validation_error!("Invalid {}: '{}'", field_name, value);
/// ```
#[macro_export]
macro_rules! validation_error {
    ($msg:literal) => {
        $crate::NebulaError::new_static(
            $crate::ErrorKind::Client($crate::kinds::ClientError::Validation {
                message: $msg.into(),
            }),
            $msg,
        )
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::NebulaError::validation(format!($fmt, $($arg)*))
    };
}

/// Create a not found error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::not_found_error;
///
/// let error = not_found_error!("User", "user-123");
/// let error = not_found_error!("Resource {} with id {}", "User", "123");
/// ```
#[macro_export]
macro_rules! not_found_error {
    ($resource_type:expr, $resource_id:expr) => {
        $crate::NebulaError::not_found($resource_type, $resource_id)
    };
    ($fmt:expr, $($arg:tt)*) => {{
        let msg = format!($fmt, $($arg)*);
        $crate::NebulaError::new_static(
            $crate::ErrorKind::Client($crate::kinds::ClientError::NotFound {
                resource_type: "unknown".into(),
                resource_id: msg.clone(),
            }),
            "Resource not found",
        ).with_details(&msg)
    }};
}

/// Create an internal server error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::internal_error;
///
/// let error = internal_error!("Database connection failed");
/// let error = internal_error!("Failed to process {} items", 42);
/// ```
#[macro_export]
macro_rules! internal_error {
    ($msg:literal) => {
        $crate::NebulaError::new_static(
            $crate::ErrorKind::Server($crate::kinds::ServerError::Internal {
                message: $msg.into(),
            }),
            $msg,
        )
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::NebulaError::internal(format!($fmt, $($arg)*))
    };
}

/// Create a timeout error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::timeout_error;
/// use std::time::Duration;
///
/// let error = timeout_error!("API call", Duration::from_secs(30));
/// let error = timeout_error!("Operation {} timed out", "process_batch");
/// ```
#[macro_export]
macro_rules! timeout_error {
    ($operation:expr, $duration:expr) => {
        $crate::NebulaError::timeout($operation, $duration)
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::NebulaError::timeout(
            format!($fmt, $($arg)*),
            std::time::Duration::from_secs(30),
        )
    };
}

/// Create a service unavailable error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::service_unavailable_error;
///
/// let error = service_unavailable_error!("database", "connection pool exhausted");
/// let error = service_unavailable_error!("Service {} is down: {}", "auth", "maintenance");
/// ```
#[macro_export]
macro_rules! service_unavailable_error {
    ($service:expr, $reason:expr) => {
        $crate::NebulaError::service_unavailable($service, $reason)
    };
    ($fmt:expr, $($arg:tt)*) => {{
        let msg = format!($fmt, $($arg)*);
        $crate::NebulaError::service_unavailable("unknown", msg)
    }};
}

/// Create a permission denied error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::permission_denied_error;
///
/// let error = permission_denied_error!("read", "sensitive_document");
/// let error = permission_denied_error!("User cannot {} resource {}", "delete", "workflow");
/// ```
#[macro_export]
macro_rules! permission_denied_error {
    ($operation:expr, $resource:expr) => {
        $crate::NebulaError::permission_denied($operation, $resource)
    };
    ($fmt:expr, $($arg:tt)*) => {{
        let msg = format!($fmt, $($arg)*);
        $crate::NebulaError::permission_denied("unknown", msg)
    }};
}

/// Create a rate limit exceeded error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::rate_limit_error;
/// use std::time::Duration;
///
/// let error = rate_limit_error!(100, Duration::from_secs(60));
/// ```
#[macro_export]
macro_rules! rate_limit_error {
    ($limit:expr, $period:expr) => {
        $crate::NebulaError::rate_limit_exceeded($limit, $period)
    };
}

/// Create an authentication error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::auth_error;
///
/// let error = auth_error!("Invalid credentials");
/// let error = auth_error!("Token expired for user {}", "john_doe");
/// ```
#[macro_export]
macro_rules! auth_error {
    ($msg:literal) => {
        $crate::NebulaError::new_static(
            $crate::ErrorKind::Client($crate::kinds::ClientError::Authentication {
                reason: $msg.into(),
            }),
            $msg,
        )
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::NebulaError::authentication(format!($fmt, $($arg)*))
    };
}

/// Create a workflow-specific error with formatted message
///
/// # Examples
///
/// ```rust
/// use nebula_error::workflow_error;
///
/// let error = workflow_error!("not_found", "user-onboarding");
/// let error = workflow_error!("execution_failed", "node-{}", "send-email");
/// ```
#[macro_export]
macro_rules! workflow_error {
    ("not_found", $workflow_id:expr) => {
        $crate::NebulaError::workflow_not_found($workflow_id)
    };
    ("disabled", $workflow_id:expr) => {
        $crate::NebulaError::workflow_disabled($workflow_id)
    };
    ("execution_failed", $fmt:expr, $($arg:tt)*) => {{
        let node_id = format!($fmt, $($arg)*);
        $crate::NebulaError::node_execution_failed(node_id, "execution failed")
    }};
}

/// Convenience macro for creating errors with context
///
/// # Examples
///
/// ```rust
/// use nebula_error::{error_with_context, ErrorContext};
///
/// let error = error_with_context!(
///     validation_error!("Invalid input"),
///     ErrorContext::new("Processing user request")
///         .with_user_id("user-123")
///         .with_request_id("req-456")
/// );
/// ```
#[macro_export]
macro_rules! error_with_context {
    ($error:expr, $context:expr) => {
        $error.with_context($context)
    };
}

/// Ensure a condition is true or return an error
///
/// # Examples
///
/// ```rust
/// use nebula_error::{ensure, validation_error};
///
/// fn validate_age(age: u32) -> nebula_error::Result<()> {
///     ensure!(age >= 18, validation_error!("Age must be at least 18"));
///     ensure!(age <= 120, validation_error!("Age must be less than 120"));
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! ensure {
    ($condition:expr, $error:expr) => {
        if !($condition) {
            return Err($error);
        }
    };
}

/// Convenience macro for creating custom errors with retry information
///
/// # Examples
///
/// ```rust
/// use nebula_error::retryable_error;
/// use std::time::Duration;
///
/// let error = retryable_error!(
///     internal_error!("Temporary failure"),
///     true,
///     Duration::from_millis(500)
/// );
/// ```
#[macro_export]
macro_rules! retryable_error {
    ($error:expr, $retryable:expr, $retry_after:expr) => {
        $error.with_retry_info($retryable, Some($retry_after))
    };
    ($error:expr, $retryable:expr) => {
        $error.with_retry_info($retryable, None)
    };
}

/// Create a memory-related error
///
/// # Examples
///
/// ```rust
/// use nebula_error::memory_error;
///
/// let error = memory_error!("allocation_failed", 1024, 8);
/// let error = memory_error!("pool_exhausted", "main_pool", 100);
/// ```
#[macro_export]
macro_rules! memory_error {
    ("allocation_failed", $size:expr, $align:expr) => {
        $crate::NebulaError::memory_allocation_failed($size, $align)
    };
    ("pool_exhausted", $pool_id:expr, $capacity:expr) => {
        $crate::NebulaError::memory_pool_exhausted($pool_id, $capacity)
    };
    ("cache_miss", $key:expr) => {
        $crate::NebulaError::memory_cache_miss($key)
    };
    ("budget_exceeded", $used:expr, $limit:expr) => {
        $crate::NebulaError::memory_budget_exceeded($used, $limit)
    };
}

/// Create a resource-related error
///
/// # Examples
///
/// ```rust
/// use nebula_error::resource_error;
///
/// let error = resource_error!("unavailable", "database", "maintenance mode", true);
/// let error = resource_error!("pool_exhausted", "http_pool", 10, 10, 5);
/// ```
#[macro_export]
macro_rules! resource_error {
    ("unavailable", $resource_id:expr, $reason:expr, $retryable:expr) => {
        $crate::NebulaError::resource_unavailable($resource_id, $reason, $retryable)
    };
    ("pool_exhausted", $resource_id:expr, $current_size:expr, $max_size:expr, $waiters:expr) => {
        $crate::NebulaError::resource_pool_exhausted(
            $resource_id,
            $current_size,
            $max_size,
            $waiters,
        )
    };
    ("health_check_failed", $resource_id:expr, $attempt:expr, $reason:expr) => {
        $crate::NebulaError::resource_health_check_failed($resource_id, $attempt, $reason)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NebulaError;

    #[test]
    fn test_validation_error_macro() {
        let error = validation_error!("Invalid input");
        assert_eq!(error.error_code(), "VALIDATION_ERROR");
        assert!(!error.is_retryable());

        let field = "email";
        let error = validation_error!("Invalid {}", field);
        assert!(error.user_message().contains("email"));
    }

    #[test]
    fn test_not_found_error_macro() {
        let error = not_found_error!("User", "123");
        assert_eq!(error.error_code(), "NOT_FOUND_ERROR");
        assert!(!error.is_retryable());
    }

    #[test]
    fn test_internal_error_macro() {
        let error = internal_error!("Database connection failed");
        assert_eq!(error.error_code(), "INTERNAL_ERROR");
        assert!(!error.is_retryable());

        let count = 42;
        let error = internal_error!("Failed to process {} items", count);
        assert!(error.user_message().contains("42"));
    }

    #[test]
    fn test_timeout_error_macro() {
        let duration = std::time::Duration::from_secs(30);
        let error = timeout_error!("API call", duration);
        assert_eq!(error.error_code(), "TIMEOUT_ERROR");
        assert!(error.is_retryable());
    }

    #[test]
    fn test_ensure_macro() {
        fn validate_age(age: u32) -> crate::Result<()> {
            ensure!(age >= 18, validation_error!("Age must be at least 18"));
            Ok(())
        }

        assert!(validate_age(20).is_ok());
        assert!(validate_age(16).is_err());
    }

    #[test]
    fn test_memory_error_macros() {
        let error = memory_error!("allocation_failed", 1024, 8);
        assert_eq!(error.error_code(), "MEMORY_ALLOCATION_FAILED");

        let error = memory_error!("cache_miss", "user:123");
        assert_eq!(error.error_code(), "MEMORY_CACHE_MISS");
    }

    #[test]
    fn test_retryable_error_macro() {
        let base_error = internal_error!("Temporary failure");
        let error = retryable_error!(base_error, true, std::time::Duration::from_secs(5));
        assert!(error.is_retryable());
        assert_eq!(error.retry_after(), Some(std::time::Duration::from_secs(5)));
    }
}
