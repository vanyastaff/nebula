//! Core error types and structures
//!
//! This module contains the fundamental error handling components:
//! - [`error`](crate::core::error) - Main [`NebulaError`](crate::NebulaError) struct and core functionality
//! - [`result`](crate::core::result) - Result type and extension traits
//! - [`traits`](crate::core::traits) - Common traits for error handling
//! - [`context`](crate::core::context) - Rich error context with metadata
//! - [`conversion`](crate::core::conversion) - Error conversion utilities
//! - [`retry`](crate::core::retry) - Retry strategies and policies

pub mod context;
pub mod conversion;
pub mod error;
pub mod result;
pub mod retry;
pub mod traits;


// Re-export core types
pub use context::{ErrorContext, ErrorContextBuilder};
pub use conversion::IntoNebulaError;
pub use error::NebulaError;
pub use result::{NebulaResultExt, Result, ResultExt};
pub use retry::{RetryStrategy, Retryable, retry, retry_with_timeout};
pub use traits::*;


