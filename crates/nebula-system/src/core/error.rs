//! Error handling for nebula-system
//!
//! This module provides unified error handling using `NebulaError` from the nebula-error crate.
//! All system operations return `SystemResult<T>` which is an alias for `Result<T, NebulaError>`.

use nebula_error::{NebulaError, Result as NebulaResult};

/// Type alias for Result with `NebulaError` for system operations
pub type SystemResult<T> = NebulaResult<T>;

// ==================== System-specific NebulaError Extensions ====================

/// Extension trait for creating system-specific `NebulaErrors`
pub trait SystemError {
    /// Create a platform-specific error
    fn system_platform_error(message: impl Into<String>, code: Option<i32>) -> Self;

    /// Create a feature not supported error
    fn system_not_supported(feature: impl Into<String>) -> Self;

    /// Create a resource not found error
    fn system_not_found(resource: impl Into<String>) -> Self;

    /// Create a permission denied error
    fn system_permission_denied(operation: impl Into<String>) -> Self;

    /// Create a memory operation error
    fn system_memory_error(operation: impl Into<String>, reason: impl Into<String>) -> Self;

    /// Create a system information parsing error
    fn system_parse_error(data_type: impl Into<String>, reason: impl Into<String>) -> Self;

    /// Create a timeout error
    fn system_timeout(operation: impl Into<String>) -> Self;

    /// Create a hardware detection error
    fn system_hardware_error(component: impl Into<String>, reason: impl Into<String>) -> Self;
}

impl SystemError for NebulaError {
    /// Create a platform-specific error
    fn system_platform_error(message: impl Into<String>, code: Option<i32>) -> Self {
        let msg = match code {
            Some(code) => format!("Platform error [{}]: {}", code, message.into()),
            None => format!("Platform error: {}", message.into()),
        };
        Self::internal(msg)
    }

    /// Create a feature not supported error
    fn system_not_supported(feature: impl Into<String>) -> Self {
        Self::internal(format!(
            "Feature not supported on this platform: {}",
            feature.into()
        ))
    }

    /// Create a resource not found error
    fn system_not_found(resource: impl Into<String>) -> Self {
        Self::not_found("system-resource", resource.into())
    }

    /// Create a permission denied error
    fn system_permission_denied(operation: impl Into<String>) -> Self {
        Self::permission_denied(operation.into(), "system-resource")
    }

    /// Create a memory operation error
    fn system_memory_error(operation: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::internal(format!(
            "Memory operation '{}' failed: {}",
            operation.into(),
            reason.into()
        ))
    }

    /// Create a system information parsing error
    fn system_parse_error(data_type: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::validation(format!(
            "Failed to parse {}: {}",
            data_type.into(),
            reason.into()
        ))
    }

    /// Create a timeout error
    fn system_timeout(operation: impl Into<String>) -> Self {
        Self::timeout(operation.into(), std::time::Duration::from_secs(30))
    }

    /// Create a hardware detection error
    fn system_hardware_error(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::internal(format!(
            "Hardware component '{}' error: {}",
            component.into(),
            reason.into()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_error_ext() {
        let error = NebulaError::system_platform_error("Invalid syscall", Some(22));
        assert!(error.is_server_error());
        assert!(error.user_message().contains("Platform error [22]"));

        let error = NebulaError::system_not_supported("temperature sensors");
        assert!(error.is_server_error());
        assert_eq!(error.error_code(), "INTERNAL_ERROR");

        let error = NebulaError::system_memory_error("allocate", "out of memory");
        assert!(error.is_server_error());
        assert!(
            error
                .user_message()
                .contains("Memory operation 'allocate' failed")
        );

        let error = NebulaError::system_permission_denied("read /proc/stat");
        assert!(error.is_client_error());
        assert_eq!(error.error_code(), "PERMISSION_DENIED_ERROR");
    }
}
