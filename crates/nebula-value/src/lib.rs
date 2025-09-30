#![allow(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(clippy::all)]
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!         .as_int()
//!         .get(key)
//!         .ok_or_else(|| NebulaError::validation("Expected integer value".to_string()))
//!         .ok_or_else(|| NebulaError::validation(format!("Expected object, got {}", data.type_name())))?
//!         .ok_or_else(|| NebulaError::validation(format!("Missing key: {}", key)))?
//!         Value::int(95), Value::int(87), Value::int(92)
//!     ("age".to_string(), Value::int(30)),
//!     ("name".to_string(), Value::string("Alice")),
//!     ("scores".to_string(), Value::array(vec![
//!     ])),
//!     assert!(items.is_array());
//!     data.as_object()
//!     println!("Found {} items", items.as_array().unwrap().len());
//! # #[cfg(feature = "serde")]
//! # #[cfg(feature = "serde")]
//! # Nebula Value - Production-Ready Value System
//! # use nebula_value::Value;
//! # {
//! # {
//! # }
//! # }
//! ## Advanced Features
//! ## Core Design Principles
//! ## Feature Flags
//! ## Performance Characteristics
//! ## Quick Start
//! ## Supported Value Types
//! ## Working with JSON
//! ### Error Handling
//! ### High-Precision Decimals
//! ### Temporal Types
//! - **Arc-based sharing**: Efficient cloning of large collections
//! - **Ergonomic**: Intuitive constructors, path-based access, and safe conversions
//! - **Extensible**: Feature flags for optional functionality without bloat
//! - **Minimal allocations**: Optimized memory usage patterns
//! - **Performance**: Minimal allocations with Arc-based sharing for large data
//! - **Predictable Serialization**: Well-defined JSON representation for all types
//! - **Type Safety**: Strong typing with [`ValueKind`] classification and compatibility rules
//! - **Type-guided operations**: Fast path for compatible type operations
//! - **Zero-copy access**: References to internal data without cloning
//! - [`core`] - Core value system and utilities
//! - [`types`] - Concrete type implementations
//! - `serde` (default): JSON serialization support
//! - `std` (default): Standard library support
//! // Always available - no feature flag needed
//! // Build complex structures
//! // Create values using ergonomic constructors
//! // ISO 8601 formatting in JSON
//! // JSON roundtrip with predictable results
//! // JSON serializes as string to preserve precision
//! // Safe access and conversion
//! // Type checking and classification
//! A comprehensive, type-safe value model designed for cross-crate data interchange
//! See individual modules for detailed documentation:
//! ].into_iter().collect());
//! ```
//! ```
//! ```
//! ```
//! ```
//! ```rust
//! ```rust
//! ```rust
//! ```rust
//! ```rust
//! assert!(num.is_numeric());
//! assert!(precise.is_decimal());
//! assert_eq!(ValueKind::Integer.code(), 'i');
//! assert_eq!(ValueKind::from_value(&num), ValueKind::Integer);
//! assert_eq!(date.to_string(), "2023-12-25");
//! assert_eq!(json, serde_json::Value::String("123.456789012345".to_string()));
//! assert_eq!(time.to_string(), "14:30:00");
//! assert_eq!(value, roundtrip);
//! fn safe_access(data: &Value, key: &str) -> ValueResult<i64> {
//! if let Some(items) = data.as_object().and_then(|o| o.get("items")) {
//! in the Nebula ecosystem. Provides a unified [`Value`] type with predictable
//! let array = Value::array(vec![num, text]);
//! let data = Value::object(map);
//! let date = Value::Date(Date::new(2023, 12, 25).unwrap());
//! let duration = Value::Duration(Duration::from_hours(2));
//! let flag = Value::bool(true);
//! let json = serde_json::Value::from(precise);
//! let json_value = serde_json::Value::from(value.clone());
//! let mut map = std::collections::HashMap::new();
//! let num = Value::int(42);
//! let precise = Value::decimal(Decimal::from_str_exact("123.456789012345").unwrap());
//! let roundtrip = Value::try_from(json_value).unwrap();
//! let text = Value::string("hello");
//! let time = Value::Time(Time::new(14, 30, 0).unwrap());
//! let value = Value::object([
//! map.insert("active".to_string(), flag);
//! map.insert("items".to_string(), array);
//! serialization, ergonomic APIs, and performance-focused design.
//! use nebula_value::{Value, Date, Time, DateTime, Duration};
//! use nebula_value::{Value, Decimal};
//! use nebula_value::{Value, ValueKind, Object, Array};
//! use nebula_value::{Value, ValueResult, NebulaError};
//! | **Collections** | | |
//! | **Numeric** | | |
//! | **Primitives** | | |
//! | **Special** | | |
//! | **Temporal** | | |
//! | Type | Description | JSON Representation |
//! | `Array` | Ordered sequences | `[1, 2, 3]` |
//! | `Boolean` | True/false values | `true`/`false` |
//! | `Bytes` | Binary data | `"<base64>"` |
//! | `DateTime` | Date + time + timezone | `"2023-12-25T14:30:00Z"` |
//! | `Date` | Calendar dates | `"2023-12-25"` |
//! | `Decimal` | High-precision numbers | `"123.456789"` |
//! | `Duration` | Time spans | `"PT1H30M"` |
//! | `File` | File references | `"<metadata>"` |
//! | `Float` | 64-bit floating point | `3.14` |
//! | `Integer` | 64-bit signed integers | `42` |
//! | `Null` | Absence of value | `null` |
//! | `Object` | Key-value mappings | `{"key": "value"}` |
//! | `String` | UTF-8 text | `"hello"` |
//! | `Time` | Time of day | `"14:30:00"` |
//! |------|-------------|-------------------|
//! }
//! }
extern crate alloc;
pub mod core;
pub mod types;
// Re-export core types
pub use core::{
    error::{ValueResult, ValueErrorExt, ValueResultExt},
    kind::{TypeCompatibility, ValueKind},
    limits::ValueLimits,
    path::{PathSegment, ValuePath},
    value::{Value, HashableValue},
    NebulaError, NebulaResult, ResultExt, // Unified error handling
};
// Re-export type implementations
pub use types::*;
/// Prelude for common imports
pub mod prelude {
    pub use crate::core::prelude::*;
    pub use crate::{Value, ValueKind, NebulaError, ValueResult, ValueErrorExt, ValueResultExt};
}
