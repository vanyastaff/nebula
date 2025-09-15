//!
//! Core building blocks for the Nebula Value model.
//!
//! This module contains the fundamental types and utilities:
//! - [`value`]: the [`Value`] enum and associated helpers.
//! - [`kind`]: the [`ValueKind`] classification and type-compatibility logic.
//! - [`error`]: strongly-typed error enums used by value operations.
//! - [`path`]: helpers to navigate nested values using paths.
//!
//! Most users will interact with re-exported items directly from the crate root.

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
