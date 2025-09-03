//! Error types for system operations

use thiserror::Error;

/// Main error type for system operations
#[derive(Error, Debug)]
pub enum SystemError {
    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Platform-specific error
    #[error("Platform error: {message}")]
    PlatformError {
        /// Error message
        message: String,
        /// OS error code if available
        code: Option<i32>,
    },

    /// Feature not supported on this platform
    #[error("Not supported on this platform: {0}")]
    NotSupported(String),

    /// Resource not found
    #[error("Resource not found: {0}")]
    NotFound(String),

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Invalid input
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Memory error
    #[cfg(feature = "memory")]
    #[error("Memory error: {0}")]
    Memory(String),

    /// Parse error
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Timeout
    #[error("Operation timed out")]
    Timeout,

    /// Custom error
    #[error("{0}")]
    Custom(String),
}

/// Result type for system operations
pub type Result<T> = std::result::Result<T, SystemError>;

impl SystemError {
    /// Create a platform error from OS error
    pub fn from_os_error(err: &std::io::Error) -> Self {
        Self::PlatformError { message: err.to_string(), code: err.raw_os_error() }
    }

    /// Check if error is recoverable
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Timeout | Self::NotFound(_) | Self::Custom(_))
    }
}

#[cfg(feature = "memory")]
impl From<region::Error> for SystemError {
    fn from(err: region::Error) -> Self {
        Self::Memory(err.to_string())
    }
}
