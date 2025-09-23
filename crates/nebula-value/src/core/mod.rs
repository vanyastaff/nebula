//! Core building blocks for the Nebula Value model.
//!
//! This module contains the fundamental types and utilities that power the value system:
//!
//! ## Core Components
//!
//! ### [`value`] - The Value enum
//! The central [`Value`] enum represents any data value in the Nebula ecosystem.
//! It supports all primitive, collection, and temporal types with efficient
//! Arc-based cloning for large data structures.
//!
//! ### [`kind`] - Type classification
//! The [`ValueKind`] system provides:
//! - Type classification and compatibility checking
//! - Category-based operations (numeric, temporal, collection, etc.)
//! - Type codes for serialization and debugging
//!
//! ### [`error`] - Comprehensive error handling
//! Strongly-typed error enums that cover:
//! - Type mismatches and conversion failures
//! - Access errors (invalid keys, indices, paths)
//! - Validation and parsing failures
//! - Operation-specific errors with context
//!
//! ### [`path`] - Value navigation
//! Path-based navigation for nested values:
//! - Dot notation and array indexing
//! - Safe traversal with error handling
//! - Mutable and immutable access patterns
//!
//! ## Usage
//! Most users interact with re-exported items from the crate root, but this
//! module provides direct access for advanced use cases.

// Core modules
pub mod error;
pub mod kind;
pub mod path;
pub mod value;

/// Convenient re-exports of the most commonly used core types.
pub use error::{ValueError, ValueResult};
pub use kind::{TypeCompatibility, ValueKind};
pub use path::{PathSegment, ValuePath};
pub use value::Value;

/// A dynamic error result type alias for ad-hoc usage.
pub type DynResult<T> = Result<T, Box<dyn std::error::Error>>;

/// A small prelude to import frequently used types in one go.
pub mod prelude {
    pub use super::{PathSegment, Value, ValueError, ValueKind, ValuePath, ValueResult};
    /// Re-export commonly used types from the `types` module.
    pub use crate::types::*;
}
