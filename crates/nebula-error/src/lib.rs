//! # Nebula Error Handling
//!
//! Centralized error handling system for the Nebula workflow engine.
//! This crate provides a unified error type system with proper error classification,
//! context propagation, and retry logic.
//!
//! ## Key Features
//!
//! - **Unified Error Types**: Single `NebulaError` enum for all errors
//! - **Error Classification**: Automatic categorization of retryable vs terminal errors
//! - **Context Propagation**: Rich error context with `.context()` method
//! - **Retry Logic**: Built-in retry strategies with exponential backoff
//! - **Error Conversion**: Automatic conversion from external error types
//!
//! ## Usage
//!
//! ```rust
//! use nebula_error::{NebulaError, Result, ErrorKind};
//!
//! fn process_data() -> Result<()> {
//!     // Your logic here
//!     // if something_went_wrong {
//!     //     return Err(NebulaError::validation("Invalid data format"));
//!     // }
//!     Ok(())
//! }
//!
//! // With context
//! let result = process_data();
//!
//! // Check if retryable
//! if let Err(e) = &result {
//!     if e.is_retryable() {
//!         // Implement retry logic
//!     }
//! }
//! ```

pub mod context;
pub mod conversion;
pub mod error;
pub mod retry;

// Re-export main types
pub use context::ErrorContext;
pub use error::{ErrorKind, NebulaError, Result};
pub use retry::{RetryStrategy, Retryable};

/// Common prelude for error handling
pub mod prelude {
    pub use super::{ErrorContext, ErrorKind, NebulaError, Result, RetryStrategy, Retryable};
}
