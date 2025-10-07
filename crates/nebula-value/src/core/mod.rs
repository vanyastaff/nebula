// Core modules
//!
//!
//!
//!
//!
//!
//!
//! ## Core Components
//! ## Usage
//! ### [`error`] - Comprehensive error handling
//! ### [`kind`] - Type classification
//! ### [`path`] - Value navigation
//! ### [`value`] - The Value enum
//! - Access errors (invalid keys, indices, paths)
//! - Category-based operations (numeric, temporal, collection, etc.)
//! - Dot notation and array indexing
//! - Mutable and immutable access patterns
//! - Operation-specific errors with context
//! - Safe traversal with error handling
//! - Type classification and compatibility checking
//! - Type codes for serialization and debugging
//! - Type mismatches and conversion failures
//! - Validation and parsing failures
//! Arc-based cloning for large data structures.
//! Core building blocks for the Nebula Value model.
//! It supports all primitive, collection, and temporal types with efficient
//! Most users interact with re-exported items from the crate root, but this
//! Path-based navigation for nested values:
//! Strongly-typed error enums that cover:
//! The [`ValueKind`] system provides:
//! The central [`Value`] enum represents any data value in the Nebula ecosystem.
//! This module contains the fundamental types and utilities that power the value system:
//! module provides direct access for advanced use cases.
pub mod conversions;
pub mod convert;
pub mod display;
pub mod error;
pub mod hash;
pub mod kind;
pub mod limits;
pub mod ops;
pub mod path;
#[cfg(feature = "serde")]
pub mod serde;
pub mod value;


pub use conversions::ValueConversion;
/// Convenient re-exports of the most commonly used core types.
pub use error::{ValueErrorExt, ValueResult, ValueResultExt};
pub use hash::{HashableValue, HashableValueExt};
pub use kind::ValueKind;
pub use path::PathSegment;
pub use value::Value;

/// Re-export NebulaError for unified error handling
pub use nebula_error::{NebulaError, Result as NebulaResult, ResultExt};

/// A dynamic error result type alias for ad-hoc usage.
pub type DynResult<T> = Result<T, Box<dyn std::error::Error>>;

/// A small prelude to import frequently used types in one go.
pub mod prelude {
    pub use super::{
        NebulaError, NebulaResult, PathSegment, Value, ValueErrorExt, ValueResult, ValueResultExt,
    };
    pub use nebula_error::ErrorContext;
}


