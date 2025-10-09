//! Error kind definitions organized by category
//!
//! This module contains all the specific error variants organized into logical
//! categories for better maintainability and understanding.
//!
//! ## Error Categories
//!
//! ### Client Errors (4xx equivalent)
//! - [`ClientError`] - User-facing errors, validation failures, not found, permission denied
//! - **Not retryable** by default (except authentication for token refresh)
//! - Indicate problems with the request that need user intervention
//!
//! ### Server Errors (5xx equivalent)
//! - [`ServerError`] - Internal server issues, service unavailable, configuration errors
//! - **Often retryable** - transient failures that may resolve on retry
//! - Indicate problems on the server side
//!
//! ### System Errors
//! - [`SystemError`] - Infrastructure issues: network, database, timeouts, rate limits
//! - **Usually retryable** - temporary resource constraints
//! - Indicate problems with external dependencies
//!
//! ### Workflow-Specific Errors
//! - [`WorkflowError`] - Workflow definition and execution errors
//! - [`NodeError`] - Individual node execution failures
//! - [`TriggerError`] - Webhook, cron, and event trigger errors
//! - [`ConnectorError`] - External service integration failures
//! - [`CredentialError`] - Authentication and credential management
//! - [`ExecutionError`] - Runtime limits, cancellation, queue overflow
//!
//! ## Design Principles
//!
//! 1. **Clear Categorization**: Errors grouped by domain for easy navigation
//! 2. **Future-Proof**: All enums marked `#[non_exhaustive]` for backward compatibility
//! 3. **Retry Logic**: Built-in retry eligibility based on error type
//! 4. **Error Codes**: Unique codes for programmatic handling and logging
//!
//! ## Usage
//!
//! ```rust
//! use nebula_error::{NebulaError, ErrorKind};
//! use nebula_error::kinds::{ClientError, ServerError};
//!
//! // Client error - not retryable
//! let err = NebulaError::validation("Invalid email format");
//! assert!(!err.is_retryable());
//! assert!(err.is_client_error());
//!
//! // Server error - retryable
//! let err = NebulaError::service_unavailable("database", "connection pool exhausted");
//! assert!(err.is_retryable());
//! assert!(err.is_server_error());
//! ```

pub mod client;
pub mod codes;
pub mod server;
pub mod system;
pub mod workflow;

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

pub use client::ClientError;
pub use server::ServerError;
pub use system::SystemError;
pub use workflow::{
    ConnectorError, CredentialError, ExecutionError, NodeError, TriggerError, WorkflowError,
};

use crate::core::traits::{ErrorClassification, ErrorCode, RetryableError};

/// Main error kind enum that categorizes all possible errors
///
/// TODO(refactor): Consider splitting into more granular error hierarchies for better type safety
/// TODO(feature): Add HTTP status code mapping for web API integration
#[non_exhaustive]
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

    /// Workflow definition and orchestration errors
    #[error(transparent)]
    Workflow(#[from] WorkflowError),

    /// Individual workflow node execution errors
    #[error(transparent)]
    Node(#[from] NodeError),

    /// Workflow trigger errors (webhook, cron, manual, event)
    #[error(transparent)]
    Trigger(#[from] TriggerError),

    /// External service connector errors
    #[error(transparent)]
    Connector(#[from] ConnectorError),

    /// Credential and authentication errors
    #[error(transparent)]
    Credential(#[from] CredentialError),

    /// Runtime execution errors (limits, cancellation, etc.)
    #[error(transparent)]
    Execution(#[from] ExecutionError),
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
            ErrorKind::Workflow(e) => e.is_retryable(),
            ErrorKind::Node(e) => e.is_retryable(),
            ErrorKind::Trigger(e) => e.is_retryable(),
            ErrorKind::Connector(e) => e.is_retryable(),
            ErrorKind::Credential(e) => e.is_retryable(),
            ErrorKind::Execution(e) => e.is_retryable(),
        }
    }

    fn retry_delay(&self) -> Option<Duration> {
        match self {
            ErrorKind::Client(e) => e.retry_delay(),
            ErrorKind::Server(e) => e.retry_delay(),
            ErrorKind::System(e) => e.retry_delay(),
            ErrorKind::Workflow(_) => None,
            ErrorKind::Node(_) => Some(Duration::from_secs(1)),
            ErrorKind::Trigger(_) => Some(Duration::from_secs(5)),
            ErrorKind::Connector(_) => Some(Duration::from_secs(2)),
            ErrorKind::Credential(_) => Some(Duration::from_secs(10)),
            ErrorKind::Execution(_) => Some(Duration::from_millis(500)),
        }
    }
}

impl ErrorCode for ErrorKind {
    fn error_code(&self) -> &str {
        match self {
            ErrorKind::Client(e) => e.error_code(),
            ErrorKind::Server(e) => e.error_code(),
            ErrorKind::System(e) => e.error_code(),
            ErrorKind::Workflow(e) => e.error_code(),
            ErrorKind::Node(e) => e.error_code(),
            ErrorKind::Trigger(e) => e.error_code(),
            ErrorKind::Connector(e) => e.error_code(),
            ErrorKind::Credential(e) => e.error_code(),
            ErrorKind::Execution(e) => e.error_code(),
        }
    }

    fn error_category(&self) -> &'static str {
        match self {
            ErrorKind::Client(_) => codes::CATEGORY_CLIENT,
            ErrorKind::Server(_) => codes::CATEGORY_SERVER,
            ErrorKind::System(_) => codes::CATEGORY_SYSTEM,
            ErrorKind::Workflow(_) => codes::CATEGORY_WORKFLOW,
            ErrorKind::Node(_) => codes::CATEGORY_NODE,
            ErrorKind::Trigger(_) => codes::CATEGORY_TRIGGER,
            ErrorKind::Connector(_) => codes::CATEGORY_CONNECTOR,
            ErrorKind::Credential(_) => codes::CATEGORY_CREDENTIAL,
            ErrorKind::Execution(_) => codes::CATEGORY_EXECUTION,
        }
    }
}

// Backwards compatibility - keep the old error variants as type aliases
pub use client::ClientError::Authentication;
pub use client::ClientError::Authorization;
pub use client::ClientError::InvalidInput;
pub use client::ClientError::NotFound;
pub use client::ClientError::PermissionDenied;
pub use client::ClientError::Validation;

pub use server::ServerError::Internal;
pub use server::ServerError::ServiceUnavailable;

pub use system::SystemError::Database;
pub use system::SystemError::ExternalService;
pub use system::SystemError::Network;
pub use system::SystemError::RateLimitExceeded;
pub use system::SystemError::ResourceExhausted;
pub use system::SystemError::Timeout;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification() {
        let client_error = ErrorKind::Client(ClientError::Validation {
            message: "Invalid input".to_string(),
        });
        assert!(client_error.is_client_error());
        assert!(!client_error.is_server_error());
        assert!(!client_error.is_system_error());

        let server_error = ErrorKind::Server(ServerError::Internal {
            message: "Database connection failed".to_string(),
        });
        assert!(!server_error.is_client_error());
        assert!(server_error.is_server_error());
        assert!(!server_error.is_system_error());

        let system_error = ErrorKind::System(SystemError::Network {
            message: "Connection timeout".to_string(),
        });
        assert!(!system_error.is_client_error());
        assert!(!system_error.is_server_error());
        assert!(system_error.is_system_error());
    }

    #[test]
    fn test_retry_behavior() {
        let validation_error = ErrorKind::Client(ClientError::Validation {
            message: "Invalid input".to_string(),
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
            message: "Invalid input".to_string(),
        });
        assert_eq!(validation_error.error_code(), "VALIDATION_ERROR");
        assert_eq!(validation_error.error_category(), "CLIENT");

        let internal_error = ErrorKind::Server(ServerError::Internal {
            message: "Server error".to_string(),
        });
        assert_eq!(internal_error.error_code(), "INTERNAL_ERROR");
        assert_eq!(internal_error.error_category(), "SERVER");
    }
}
