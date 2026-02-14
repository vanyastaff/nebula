//! Re-exports for expression error types
//!
//! This module provides backward-compatible re-exports of error types.

// Re-export error types from the main error module
pub use crate::error::{ExpressionError, ExpressionErrorExt, ExpressionResult};
