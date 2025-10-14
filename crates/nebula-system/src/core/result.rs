//! Result type and extension traits for system operations

use crate::core::error::{SystemError, SystemResult};

/// Extension trait for Result types (system-specific)
pub trait SystemResultExt<T> {
    /// Convert to `SystemResult` with custom system error message
    fn or_system_error<S: Into<String>>(self, msg: S) -> SystemResult<T>;

    /// Add system context to error
    fn with_system_context<S: Into<String>, F>(self, f: F) -> SystemResult<T>
    where
        F: FnOnce() -> S;

    /// Add component context for system operations
    fn with_component(self, component: impl Into<String>) -> SystemResult<T>;

    /// Add operation context for system operations
    fn with_operation(self, operation: impl Into<String>) -> SystemResult<T>;

    /// Add platform-specific context
    fn with_platform_context(self, platform: impl Into<String>) -> SystemResult<T>;
}

impl<T, E> SystemResultExt<T> for Result<T, E>
where
    E: std::error::Error,
{
    fn or_system_error<S: Into<String>>(self, msg: S) -> SystemResult<T> {
        self.map_err(|_| SystemError::platform_error(msg))
    }

    fn with_system_context<S: Into<String>, F>(self, f: F) -> SystemResult<T>
    where
        F: FnOnce() -> S,
    {
        self.map_err(|e| SystemError::platform_error(format!("{}: {}", f().into(), e)))
    }

    fn with_component(self, component: impl Into<String>) -> SystemResult<T> {
        self.map_err(|e| {
            SystemError::platform_error(format!(
                "System operation failed in component {}: {}",
                component.into(),
                e
            ))
        })
    }

    fn with_operation(self, operation: impl Into<String>) -> SystemResult<T> {
        self.map_err(|e| {
            SystemError::platform_error(format!(
                "System operation {} failed: {}",
                operation.into(),
                e
            ))
        })
    }

    fn with_platform_context(self, platform: impl Into<String>) -> SystemResult<T> {
        self.map_err(|e| {
            SystemError::platform_error(format!(
                "Platform {} operation failed: {}",
                platform.into(),
                e
            ))
        })
    }
}

/// Extension trait specifically for IO Result types in system context
pub trait SystemIoResultExt {
    /// Convert IO error to `SystemResult` with custom message
    fn or_system_error<S: Into<String>>(self, msg: S) -> SystemResult<()>;

    /// Add system context to IO error
    fn with_system_context<S: Into<String>, F>(self, f: F) -> SystemResult<()>
    where
        F: FnOnce() -> S;

    /// Add component context for IO operations
    fn with_component(self, component: impl Into<String>) -> SystemResult<()>;

    /// Add operation context for IO operations
    fn with_operation(self, operation: impl Into<String>) -> SystemResult<()>;
}

// Specific implementations for common error types
impl SystemIoResultExt for Result<(), std::io::Error> {
    fn or_system_error<S: Into<String>>(self, msg: S) -> SystemResult<()> {
        self.map_err(|e| SystemError::platform_error(format!("{}: {}", msg.into(), e)))
    }

    fn with_system_context<S: Into<String>, F>(self, f: F) -> SystemResult<()>
    where
        F: FnOnce() -> S,
    {
        self.map_err(|e| SystemError::platform_error(format!("{}: {}", f().into(), e)))
    }

    fn with_component(self, component: impl Into<String>) -> SystemResult<()> {
        self.map_err(|e| {
            SystemError::platform_error(format!(
                "IO operation failed in component {}: {e}",
                component.into()
            ))
        })
    }

    fn with_operation(self, operation: impl Into<String>) -> SystemResult<()> {
        self.map_err(|e| {
            SystemError::platform_error(format!("IO operation {} failed: {e}", operation.into()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_system_io_result_ext() {
        let result: Result<(), io::Error> = Err(io::Error::new(io::ErrorKind::NotFound, "test"));
        let system_result = SystemIoResultExt::or_system_error(result, "File operation failed");

        assert!(system_result.is_err());
        let error = system_result.unwrap_err();
        assert!(matches!(error, SystemError::PlatformError(_)));
    }

    #[test]
    fn test_with_component() {
        let result: Result<(), io::Error> = Err(io::Error::new(io::ErrorKind::NotFound, "test"));
        let system_result = SystemIoResultExt::with_component(result, "cpu-info");

        assert!(system_result.is_err());
        let error = system_result.unwrap_err();
        assert!(matches!(error, SystemError::PlatformError(_)));
    }
}
