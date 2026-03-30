//! Error handling for nebula-system
//

use thiserror::Error;

/// Result type alias for system operations
pub type SystemResult<T> = Result<T, SystemError>;

/// Error types for system operations
#[derive(Error, Debug, nebula_error::Classify)]
pub enum SystemError {
    /// Platform-specific error occurred
    #[classify(category = "internal", code = "SYSTEM:PLATFORM")]
    #[error("Platform error: {0}")]
    PlatformError(String),
    /// Requested feature is not supported on this platform
    #[classify(category = "unsupported", code = "SYSTEM:UNSUPPORTED")]
    #[error("Feature not supported: {0}")]
    FeatureNotSupported(String),
    /// System resource was not found
    #[classify(category = "not_found", code = "SYSTEM:NOT_FOUND")]
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    /// Permission denied for system operation
    #[classify(category = "authorization", code = "SYSTEM:PERMISSION_DENIED")]
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    /// Memory operation failed
    #[classify(category = "internal", code = "SYSTEM:MEMORY")]
    #[error("Memory operation error: {0}")]
    MemoryOperationError(String),
    /// Failed to parse system data
    #[classify(category = "internal", code = "SYSTEM:PARSE")]
    #[error("System parse error: {0}")]
    SystemParseError(String),
    /// System operation timed out
    #[classify(category = "timeout", code = "SYSTEM:TIMEOUT")]
    #[error("System timeout: {0}")]
    SystemTimeout(String),
    /// Hardware-level error occurred
    #[classify(category = "internal", code = "SYSTEM:HARDWARE")]
    #[error("System hardware error: {0}")]
    SystemHardwareError(String),
}

impl SystemError {
    /// Create a platform error
    pub fn platform_error(message: impl Into<String>) -> Self {
        Self::PlatformError(message.into())
    }

    /// Create a feature not supported error
    pub fn feature_not_supported(message: impl Into<String>) -> Self {
        Self::FeatureNotSupported(message.into())
    }

    /// Create a resource not found error
    pub fn resource_not_found(message: impl Into<String>) -> Self {
        Self::ResourceNotFound(message.into())
    }

    /// Create a permission denied error
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::PermissionDenied(message.into())
    }

    /// Create a memory operation error
    pub fn memory_operation_error(message: impl Into<String>) -> Self {
        Self::MemoryOperationError(message.into())
    }

    /// Create a system parse error
    pub fn system_parse_error(message: impl Into<String>) -> Self {
        Self::SystemParseError(message.into())
    }

    /// Create a system timeout error
    pub fn system_timeout(message: impl Into<String>) -> Self {
        Self::SystemTimeout(message.into())
    }

    /// Create a system hardware error
    pub fn system_hardware_error(message: impl Into<String>) -> Self {
        Self::SystemHardwareError(message.into())
    }
}
