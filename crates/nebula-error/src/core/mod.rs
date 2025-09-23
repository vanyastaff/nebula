//! Core error types and structures
//!
//! This module contains the fundamental error handling components:
//! - [`error`] - Main `NebulaError` struct and core functionality
//! - [`result`] - Result type and extension traits
//! - [`traits`] - Common traits for error handling
//! - [`context`] - Rich error context with metadata
//! - [`conversion`] - Error conversion utilities
//! - [`retry`] - Retry strategies and policies

pub mod error;
pub mod result;
pub mod traits;
pub mod context;
pub mod conversion;
pub mod retry;

// Re-export core types
pub use error::NebulaError;
pub use result::{Result, ResultExt, NebulaResultExt};
pub use traits::*;
pub use context::{ErrorContext, ErrorContextBuilder};
pub use conversion::{IntoNebulaError, ResultExt as ConversionResultExt};
pub use retry::{RetryStrategy, Retryable, retry, retry_with_timeout};