//! Validation module for nebula-value
//!
//! This module provides validation utilities and limits for Value operations.

pub mod limits;

// Re-export from core (limits are actually in core/limits.rs)
pub use crate::core::limits::ValueLimits;