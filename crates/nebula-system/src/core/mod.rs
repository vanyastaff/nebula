//! Core components for the Nebula system information library.
//!
//! This module contains the fundamental types and utilities that power the system information gathering:
//!
//! ## Core Components
//!
//! ### [`error`] - Error handling
//! Unified error handling using `NebulaError` for all system operations.
//! Provides structured error types for platform-specific, I/O, and system errors.
//!
//! ### [`result`] - Result types
//! Type aliases and extension traits for working with system operation results.
//! Simplifies error handling across the system information gathering.

pub mod error;
pub mod result;

// Re-export core types
pub use error::{SystemError, SystemResult};
pub use result::SystemResultExt;

// Re-export NebulaError for unified error handling
pub use nebula_error::{NebulaError, Result as NebulaResult, ResultExt};
