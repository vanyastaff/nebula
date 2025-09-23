//! # Nebula Error Handling
//!
//! Centralized error handling system for the Nebula workflow engine.
//! This crate provides a unified error type system with proper error classification,
//! context propagation, and retry logic.
//!
//! ## Architecture
//!
//! ### Core Components
//! - [`core`] - Core error types and main `NebulaError` struct
//! - [`kinds`] - Categorized error types by domain (client, server, system, etc.)
//! - [`context`] - Rich error context with metadata and tracing information
//! - [`retry`] - Retry strategies and policies for transient failures
//! - [`conversion`] - Conversion utilities from external error types
//!
//! ## Key Features
//!
//! - **Unified Error Type**: Single `NebulaError` with structured error kinds
//! - **Error Classification**: Automatic categorization (client vs server vs system)
//! - **Rich Context**: Structured error context with metadata and correlation IDs
//! - **Retry Logic**: Built-in retry strategies with exponential backoff and jitter
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
pub use core::{NebulaError, Result, ResultExt, NebulaResultExt};
pub use kinds::ErrorKind;

// Re-export utilities from core
pub use core::{
    ErrorContext, ErrorContextBuilder,
    IntoNebulaError,
    RetryStrategy, Retryable, retry, retry_with_timeout
};

/// Common prelude for error handling
pub mod prelude {
    pub use super::{
        NebulaError, Result, ResultExt, ErrorKind,
        ErrorContext, ErrorContextBuilder,
        RetryStrategy, Retryable, retry, retry_with_timeout,
        IntoNebulaError,
    };
    pub use thiserror::Error as ThisError;
}

// Public re-export of thiserror::Error at the crate root for convenience
pub use thiserror::Error;
