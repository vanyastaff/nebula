//!
//! Nebula Value
//! ==================
//!
//! A lightweight, fast, and expressive value model used across the Nebula ecosystem.
//! It provides a dynamically-typed [`Value`] with a precise [`ValueKind`], rich
//! error types, and utilities for conversion, inspection and manipulation.
//! The crate is designed to be `no_std` friendly and integrates well with other
//! Nebula crates.
//!
//! ## Features
//! - Unified [`Value`] enum with support for:
//!   - Primitive types: booleans, text, bytes
//!   - Numeric types: integers, floats, decimals (optional)
//!   - Collection types: arrays, objects
//!   - Temporal types: dates, times, datetimes, durations
//! - [`ValueKind`] helpers for type classification and compatibility checks
//! - Comprehensive error types in [`core::error`]
//! - Path-based navigation for nested values
//! - Optional features for extended functionality
//!
//! ## Quick start
//! ```rust
//! use nebula_value::{Value, ValueKind};
//!
//! let v = Value::from(42);
//! assert_eq!(ValueKind::from_value(&v), ValueKind::Integer);
//!
//! // Type-aware operations
//! assert!(ValueKind::Integer.is_numeric());
//! assert_eq!(ValueKind::Integer.code(), 'i');
//! ```
//!
//! ## Working with collections
//! ```rust
//! use nebula_value::{Value, Array, Object};
//!
//! // Arrays
//! let arr = Value::Array(Array::from(vec![Value::from(1), Value::from(2)]));
//!
//! // Objects
//! let mut obj = Object::new();
//! obj.insert("key".to_string(), Value::from("value"));
//! let obj_val = Value::Object(obj);
//! ```
//!
//! See the [`core`] and [`types`] modules for more details.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

pub mod core;
pub mod types;

// Re-export core types
pub use core::{
    error::{ValueError, ValueResult},
    kind::{TypeCompatibility, ValueKind},
    path::{PathSegment, ValuePath},
    value::Value,
};

// Re-export type implementations
pub use types::*;
