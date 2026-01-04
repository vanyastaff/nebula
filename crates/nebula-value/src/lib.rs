//! # nebula-value
//!
//! Production-ready value type system for the Nebula workflow engine.
//!
//! ## Overview
//!
//! `nebula-value` provides a unified [`Value`] type that can represent any data
//! in the Nebula ecosystem, from simple scalars to complex nested structures.
//! It is designed for workflow automation similar to n8n, with a focus on
//! performance, type safety, and developer experience.
//!
//! ## Key Features
//!
//! - **Type Safety**: Comprehensive error handling with [`ValueError`]
//! - **Performance**: O(log n) operations with persistent data structures ([`im`])
//! - **Zero-Copy**: Arc-based cloning for efficient data sharing
//! - **Thread-Safe**: Immutable APIs with lock-free operations
//! - **DoS Protection**: Built-in limits for arrays, objects, and strings
//! - **Temporal Types**: Date, Time, DateTime, Duration with ISO 8601/RFC 3339
//! - **JSON Integration**: Seamless conversion to/from JSON with serde
//!
//! ## Quick Start
//!
//! ```rust
//! use nebula_value::prelude::*;
//!
//! // Create values
//! let num = Value::integer(42);
//! let text = Value::text("hello");
//! let flag = Value::boolean(true);
//!
//! // Operations
//! # #[cfg(feature = "serde")]
//! # {
//! let sum = num.add(&Value::integer(8))?;
//! assert_eq!(sum.to_integer()?, 50);
//! # }
//!
//! // Parse from JSON
//! # #[cfg(feature = "serde")]
//! # {
//! let value: Value = r#"{"name": "Alice", "age": 30}"#.parse()?;
//! assert!(value.is_object());
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Value Types
//!
//! The [`Value`] enum supports these variants:
//!
//! ### Scalar Types
//!
//! - [`Value::Null`] - Absence of value
//! - [`Value::Boolean`] - True/false
//! - [`Value::Integer`] - 64-bit signed integer (i64)
//! - [`Value::Float`] - 64-bit floating point (f64)
//! - [`Value::Decimal`] - Arbitrary precision decimal
//! - [`Value::Text`] - UTF-8 text with Arc<str> for efficient cloning
//! - [`Value::Bytes`] - Binary data with bytes::Bytes
//!
//! ### Collection Types
//!
//! - [`Value::Array`] - Ordered sequence using [`im::Vector`]
//! - [`Value::Object`] - Key-value map using [`im::HashMap`]
//!
//! ### Temporal Types (feature = "temporal")
//!
//! - [`Value::Date`] - Calendar date (ISO 8601)
//! - [`Value::Time`] - Time of day (ISO 8601)
//! - [`Value::DateTime`] - Date + time + timezone (RFC 3339)
//! - [`Value::Duration`] - Time span in milliseconds
//!
//! ## Architecture
//!
//! The crate is organized into focused modules:
//!
//! - [`core`] - Core [`Value`] type, operations, conversions, and error handling
//! - [`scalar`] - Scalar types: [`Integer`], [`Float`], [`Text`], [`Bytes`], [`Boolean`]
//! - [`collections`] - Collections: [`Array`], [`Object`] with builder patterns
//! - [`temporal`] - Temporal types: [`Date`], [`Time`], [`DateTime`], [`Duration`]
//! - [`error`] - Comprehensive error handling with [`ValueError`]
//!
//! ## Performance
//!
//! - **Persistent Data Structures**: O(log n) for most operations with structural sharing
//! - **Zero-Copy Cloning**: Arc-based references, no data duplication
//! - **Small Value Optimization**: Inline storage where beneficial
//! - **Thread-Safe**: Lock-free immutable operations
//!
//! ## Examples
//!
//! ### Creating Values
//!
//! ```rust
//! use nebula_value::Value;
//!
//! let null = Value::null();
//! let number = Value::integer(42);
//! let text = Value::text("hello");
//! let bytes = Value::bytes(vec![1, 2, 3]);
//! let empty_array = Value::array_empty();
//! let empty_object = Value::object_empty();
//! ```
//!
//! ### Working with Collections
//!
//! ```rust
//! use nebula_value::{Array, Object, Value};
//! use nebula_value::collections::array::ArrayBuilder;
//! use nebula_value::collections::object::ObjectBuilder;
//!
//! // Build an array
//! let array = ArrayBuilder::new()
//!     .push(Value::integer(1))
//!     .push(Value::integer(2))
//!     .push(Value::integer(3))
//!     .build()
//!     .unwrap();
//!
//! // Build an object
//! let object = ObjectBuilder::new()
//!     .insert("name", Value::text("Alice"))
//!     .insert("age", Value::integer(30))
//!     .build()
//!     .unwrap();
//! ```
//!
//! ### Type Conversions
//!
//! ```rust
//! use nebula_value::Value;
//!
//! let val = Value::integer(42);
//!
//! // Type checking
//! assert!(val.is_integer());
//! assert!(val.is_numeric());
//!
//! // Safe conversions (Option)
//! if let Some(num) = val.as_integer() {
//!     println!("Got integer: {}", num.value());
//! }
//!
//! // Fallible conversions (Result)
//! let num: i64 = val.to_integer()?;
//! assert_eq!(num, 42);
//! # Ok::<(), nebula_value::ValueError>(())
//! ```
//!
//! ## Features
//!
//! - `default = ["std", "temporal"]` - Standard library + temporal types
//! - `std` - Standard library support (enables system time methods)
//! - `temporal` - Date, Time, DateTime, Duration types
//! - `serde` - JSON serialization/deserialization
//! - `full = ["std", "serde", "temporal"]` - All features enabled
//!
//! ## See Also
//!
//! - [Repository](https://github.com/vanyastaff/nebula)
//! - [Documentation](https://docs.rs/nebula-value)
//! - [Examples](https://github.com/vanyastaff/nebula/tree/main/crates/nebula-value/examples)

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::all)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

extern crate alloc;

/// Collection types for nebula-value.
///
/// This module provides efficient persistent data structures:
/// - [`Array`] - Ordered sequence backed by [`im::Vector`]
/// - [`Object`] - Key-value map backed by [`im::HashMap`]
///
/// Both types support O(log n) operations and efficient structural sharing.
pub mod collections;

/// Core value types and operations.
///
/// This module contains:
/// - [`Value`] - The main enum representing any value
/// - Type conversions and operations
/// - Error handling and limits
/// - Serialization support
pub mod core;

/// Error types for nebula-value operations.
///
/// Contains [`ValueError`] and related error handling utilities.
pub mod error;

/// Enhanced error handling with context and suggestions.
///
/// Provides [`EnhancedError`] with detailed diagnostics, recovery hints,
/// and helpful suggestions for fixing errors.
///
/// [`EnhancedError`]: error_ext::EnhancedError
pub mod error_ext;

/// Bounded types with compile-time limits using const generics.
///
/// Provides [`BoundedText`], [`BoundedArray`], and [`BoundedObject`] with
/// compile-time maximum sizes encoded in their type signatures.
///
/// [`BoundedText`]: bounded::BoundedText
/// [`BoundedArray`]: bounded::BoundedArray
/// [`BoundedObject`]: bounded::BoundedObject
pub mod bounded;

/// Helper traits and utilities for working with Values.
///
/// Provides extension traits like [`ValueExt`], [`ArrayExt`], and [`ObjectExt`]
/// for more ergonomic value manipulation.
///
/// [`ValueExt`]: helpers::ValueExt
/// [`ArrayExt`]: helpers::ArrayExt
/// [`ObjectExt`]: helpers::ObjectExt
pub mod helpers;

/// Diff and Patch operations for comparing and updating Values.
///
/// Provides:
/// - [`ValueDiff`] - Represents a single change (added/removed/changed)
/// - [`Value::diff`] - Compute differences between values
/// - [`Value::apply_diff`] - Apply changes to produce new value
/// - [`DiffOptions`] - Control diff behavior
///
/// [`ValueDiff`]: diff::ValueDiff
/// [`DiffOptions`]: diff::DiffOptions
pub mod diff;

/// Value Schema for type introspection and validation.
///
/// Provides:
/// - [`ValueSchema`] - Describes expected value structure
/// - [`Value::infer_schema`] - Derive schema from value
/// - [`Value::matches_schema`] - Check if value matches schema
/// - [`ObjectSchema`] - Schema for object types
///
/// [`ValueSchema`]: schema::ValueSchema
/// [`ObjectSchema`]: schema::ObjectSchema
pub mod schema;

/// Iterators for traversing and transforming Value structures.
///
/// Provides:
/// - [`ValueWalker`] - Depth-first traversal iterator
/// - [`Value::walk`] - Walk all nested values
/// - [`Value::find`] - Find values by predicate
/// - [`Value::map_values`] - Transform values
/// - [`Value::filter_deep`] - Recursively filter values
///
/// [`ValueWalker`]: iter::ValueWalker
pub mod iter;

/// Scalar value types.
///
/// This module provides wrapper types for scalar values:
/// - [`Integer`] - 64-bit signed integer
/// - [`Float`] - 64-bit floating point with special NaN handling
/// - [`Text`] - UTF-8 string with Arc-based sharing
/// - [`Bytes`] - Binary data
/// - [`Boolean`] - Boolean value wrapper
pub mod scalar;

/// Temporal types for dates, times, and durations.
///
/// Available with the `temporal` feature flag.
///
/// Provides:
/// - [`Date`] - Calendar dates (ISO 8601)
/// - [`Time`] - Time of day (ISO 8601)
/// - [`DateTime`] - Date + time + timezone (RFC 3339)
/// - [`Duration`] - Time spans in milliseconds
#[cfg(feature = "temporal")]
#[cfg_attr(docsrs, doc(cfg(feature = "temporal")))]
pub mod temporal;

// Re-export core types
pub use core::{
    ConversionError, ConversionResult, Path, PathSegment, ResultExt, ValueKind,
    limits::ValueLimits, value::Value,
};

// Re-export standalone error
pub use error::{ValueError, ValueResult, ValueResultExt};

// Re-export serde-specific errors
#[cfg(feature = "serde")]
pub use core::{SerdeError, SerdeResult};

// Re-export scalar and collection types
pub use collections::{Array, Object};
pub use scalar::{Boolean, Bytes, Float, Integer, Text};

// Re-export iterator types
pub use iter::{LeavesWalker, ValueWalker};

// Re-export temporal types
#[cfg(feature = "temporal")]
pub use temporal::{Date, DateTime, Duration, Time};

// Re-export serde_json::json! macro for convenience
#[cfg(feature = "serde")]
pub use serde_json::json;

// Re-export conversion extension traits for ergonomic usage
#[cfg(feature = "serde")]
pub use core::convert::{JsonValueExt, ValueRefExt};

/// Prelude for common imports
pub mod prelude {
    pub use crate::{Array, Boolean, Bytes, Float, Integer, Object, Path, PathSegment, Text};
    pub use crate::{
        ConversionError, ConversionResult, Value, ValueError, ValueResult, ValueResultExt,
    };

    #[cfg(feature = "temporal")]
    pub use crate::{Date, DateTime, Duration, Time};

    #[cfg(feature = "serde")]
    pub use crate::{SerdeError, SerdeResult};

    #[cfg(feature = "serde")]
    pub use serde_json::json;
}
