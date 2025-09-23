//! Error kind definitions organized by category
//!
//! This module contains all the specific error variants organized into logical
//! categories for better maintainability and understanding.
//!
//! ## Categories
//! - [`client`] - Client errors (4xx equivalent) - user errors, validation, etc.
//! - [`server`] - Server errors (5xx equivalent) - internal errors, service issues
//! - [`system`] - System errors - infrastructure, network, resource issues

pub mod client;
pub mod server;
pub mod system;

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

pub use client::ClientError;
pub use server::ServerError;
pub use system::SystemError;

use crate::core::traits::{ErrorClassification, ErrorCode, RetryableError};

/// Main error kind enum that categorizes all possible errors
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum ErrorKind {
    /// Client-side errors (4xx equivalent)
    #[error(transparent)]
    Client(#[from] ClientError),

    /// Server-side errors (5xx equivalent)
    #[error(transparent)]
    Server(#[from] ServerError),

    /// System-level errors (infrastructure, network, etc.)
    #[error(transparent)]
    System(#[from] SystemError),
}

impl ErrorClassification for ErrorKind {
    fn is_client_error(&self) -> bool {
        matches!(self, ErrorKind::Client(_))
    }

    fn is_server_error(&self) -> bool {
        matches!(self, ErrorKind::Server(_))
    }

    fn is_system_error(&self) -> bool {
        matches!(self, ErrorKind::System(_))
    }
}

impl RetryableError for ErrorKind {
    fn is_retryable(&self) -> bool {
        match self {
            ErrorKind::Client(e) => e.is_retryable(),
            ErrorKind::Server(e) => e.is_retryable(),
            ErrorKind::System(e) => e.is_retryable(),
        }
    }

    fn retry_delay(&self) -> Option<Duration> {
        match self {
            ErrorKind::Client(e) => e.retry_delay(),
            ErrorKind::Server(e) => e.retry_delay(),
            ErrorKind::System(e) => e.retry_delay(),
        }
    }
}

impl ErrorCode for ErrorKind {
    fn error_code(&self) -> &str {
        match self {
            ErrorKind::Client(e) => e.error_code(),
            ErrorKind::Server(e) => e.error_code(),
            ErrorKind::System(e) => e.error_code(),
        }
    }

    fn error_category(&self) -> &str {
        match self {
            ErrorKind::Client(_) => "CLIENT",
            ErrorKind::Server(_) => "SERVER",
            ErrorKind::System(_) => "SYSTEM",
        }
    }
}

// Backwards compatibility - keep the old error variants as type aliases
pub use client::ClientError::Validation;
pub use client::ClientError::NotFound;
pub use client::ClientError::InvalidInput;
pub use client::ClientError::PermissionDenied;
pub use client::ClientError::Authentication;
pub use client::ClientError::Authorization;

pub use server::ServerError::Internal;
pub use server::ServerError::ServiceUnavailable;

pub use system::SystemError::Timeout;
pub use system::SystemError::Network;
pub use system::SystemError::Database;
pub use system::SystemError::ExternalService;
pub use system::SystemError::RateLimitExceeded;
pub use system::SystemError::ResourceExhausted;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification() {
        let client_error = ErrorKind::Client(ClientError::Validation {
            message: "Invalid input".to_string()
        });
        assert!(client_error.is_client_error());
        assert!(!client_error.is_server_error());
        assert!(!client_error.is_system_error());

        let server_error = ErrorKind::Server(ServerError::Internal {
            message: "Database connection failed".to_string()
        });
        assert!(!server_error.is_client_error());
        assert!(server_error.is_server_error());
        assert!(!server_error.is_system_error());

        let system_error = ErrorKind::System(SystemError::Network {
            message: "Connection timeout".to_string()
        });
        assert!(!system_error.is_client_error());
        assert!(!system_error.is_server_error());
        assert!(system_error.is_system_error());
    }

    #[test]
    fn test_retry_behavior() {
        let validation_error = ErrorKind::Client(ClientError::Validation {
            message: "Invalid input".to_string()
        });
        assert!(!validation_error.is_retryable());

        let timeout_error = ErrorKind::System(SystemError::Timeout {
            operation: "API call".to_string(),
            duration: Duration::from_secs(30),
        });
        assert!(timeout_error.is_retryable());
    }

    #[test]
    fn test_error_codes() {
        let validation_error = ErrorKind::Client(ClientError::Validation {
            message: "Invalid input".to_string()
        });
        assert_eq!(validation_error.error_code(), "VALIDATION_ERROR");
        assert_eq!(validation_error.error_category(), "CLIENT");

        let internal_error = ErrorKind::Server(ServerError::Internal {
            message: "Server error".to_string()
        });
        assert_eq!(internal_error.error_code(), "INTERNAL_ERROR");
        assert_eq!(internal_error.error_category(), "SERVER");
    }
}