//! Core components for the Nebula logging system.
//!
//! This module contains the fundamental types and utilities that power the logging system:
//!
//! ## Core Components
//!
//! ### [`error`] - Error handling
//! Unified error handling using [`NebulaError`] for all logging operations.
//! Provides structured error types for configuration, IO, and filter errors.
//!
//! ### [`result`] - Result types
//! Type aliases and extension traits for working with logging results.
//! Simplifies error handling across the logging system.

pub mod error;
pub mod result;

// Re-export core types
pub use error::{LogError, LogResult};
pub use result::LogResultExt;
