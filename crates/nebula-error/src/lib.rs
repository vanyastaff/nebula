//! # Nebula Error Handling
//!
//! Centralized error handling system for the Nebula workflow engine.
//! This crate provides a unified error type system with proper error classification,
//! context propagation, and retry logic for workflow orchestration.

// Strategic allows for pedantic clippy warnings that don't affect production code quality
#![allow(clippy::must_use_candidate)] // Helper methods where must_use is overly pedantic
#![allow(clippy::return_self_not_must_use)] // Builder patterns are self-documenting
#![allow(clippy::match_same_arms)] // Some match arms have distinct comments/semantics
#![allow(clippy::doc_markdown)] // Technical terms don't need backticks everywhere
#![allow(clippy::uninlined_format_args)] // Explicit format args improve readability in some cases
#![allow(clippy::missing_errors_doc)] // Error variants are self-documenting
#![allow(clippy::wildcard_imports)] // Used only in module re-exports for convenience
#![allow(clippy::cast_precision_loss)] // Duration calculations use safe ranges
#![allow(clippy::cast_possible_truncation)] // Delay calculations are bounded
#![allow(clippy::cast_sign_loss)] // Delay values are always positive
//!
//! ## Architecture
//!
//! ### Core Components
//! - [`core`](crate::core) - Core error types and main [`NebulaError`] struct
//! - [`kinds`](crate::kinds) - Categorized error types by domain (client, server, system, workflow)
//! - [`ErrorContext`](crate::ErrorContext) - Rich error context with metadata and tracing information
//! - [`retry`](crate::retry) function - Retry strategies and policies for transient failures
//! - [`IntoNebulaError`](crate::IntoNebulaError) - Conversion utilities from external error types
//!
//! ## Key Features
//!
//! - **Unified Error Type**: Single `NebulaError` with structured error kinds
//! - **Error Classification**: Automatic categorization (client vs server vs system vs workflow)
//! - **Rich Context**: Structured error context with metadata and correlation IDs
//! - **Retry Logic**: Built-in retry strategies with exponential backoff and jitter
//! - **Workflow Support**: Specialized errors for nodes, triggers, connectors, and execution
//! - **Seamless Conversion**: Automatic conversion from standard library and third-party errors
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_error::{NebulaError, Result, ResultExt};
//!
//! fn process_data() -> Result<String> {
//!     // Validation errors
//!     if true {
//!         return Err(NebulaError::validation("Invalid data format"));
//!     }
//!
//!     // With context
//!     let result = risky_operation()
//!         .context("Processing user data")?;
//!
//!     Ok(result)
//! }
//!
//! fn risky_operation() -> Result<String> {
//!     // This could be retried automatically based on error type
//!     Ok("success".to_string())
//! }
//! ```
//!
//! ## Error Categories
//!
//! ```rust
//! use nebula_error::NebulaError;
//!
//! // Client errors (4xx) - Not retryable
//! let validation_err = NebulaError::validation("Missing required field");
//! let not_found_err = NebulaError::not_found("User", "123");
//! let permission_err = NebulaError::permission_denied("read", "sensitive_data");
//!
//! // Server errors (5xx) - Often retryable
//! let service_err = NebulaError::service_unavailable("database", "connection pool exhausted");
//! let timeout_err = NebulaError::timeout("API call", std::time::Duration::from_secs(30));
//!
//! // Check retry eligibility
//! assert!(!validation_err.is_retryable());
//! assert!(service_err.is_retryable());
//! ```
//!
//! ## Workflow-Specific Errors
//!
//! ```rust
//! use nebula_error::NebulaError;
//! use std::time::Duration;
//!
//! // Workflow orchestration errors
//! let workflow_err = NebulaError::workflow_not_found("user-onboarding");
//! let node_err = NebulaError::node_execution_failed("send-email", "SMTP timeout");
//! let trigger_err = NebulaError::trigger_invalid_cron_expression("* * * * * *", "invalid format");
//!
//! // External service errors
//! let connector_err = NebulaError::connector_api_call_failed("slack", "/api/chat.postMessage", 500);
//! let credential_err = NebulaError::credential_not_found("slack-oauth-token");
//!
//! // Execution limit errors
//! let memory_err = NebulaError::execution_memory_limit_exceeded(512, 256);
//! let queue_err = NebulaError::execution_queue_full(1000, 1000);
//! ```
//!
//! ## With Retry Logic
//!
//! ```rust
//! use nebula_error::{RetryStrategy, retry};
//! use std::time::Duration;
//!
//! # async fn example() -> nebula_error::Result<()> {
//! let strategy = RetryStrategy::default()
//!     .with_max_attempts(3)
//!     .with_base_delay(Duration::from_millis(100));
//!
//! let result = retry(|| async {
//!     // Your async operation here
//!     Ok::<_, nebula_error::NebulaError>("success")
//! }, &strategy).await?;
//! # Ok(())
//! # }
//! ```

// Core modules
pub mod core;
pub mod kinds;

// Re-export core types
pub use core::{NebulaError, NebulaResultExt, Result, ResultExt};
pub use kinds::ErrorKind;

// Re-export utilities from core
pub use core::{
    ErrorContext, ErrorContextBuilder, IntoNebulaError, RetryStrategy, Retryable, retry,
    retry_with_timeout,
};

/// Common prelude for error handling
pub mod prelude {
    pub use super::{
        ErrorContext, ErrorContextBuilder, ErrorKind, IntoNebulaError, NebulaError, Result,
        ResultExt, RetryStrategy, Retryable, retry, retry_with_timeout,
    };
    pub use thiserror::Error as ThisError;
}

// Public re-export of thiserror::Error at the crate root for convenience
pub use thiserror::Error;
