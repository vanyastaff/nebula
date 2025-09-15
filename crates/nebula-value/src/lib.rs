//!
//! Nebula Value
//! ==================
//!
//! A lightweight, fast, and expressive value model used across the Nebula ecosystem.
//! It provides a dynamically-typed [`Value`] with a precise [`ValueKind`], rich
//! error types, and a small set of utilities for conversion, inspection and
//! validation. The crate is designed to be `no_std` friendly and integrates well
//! with other Nebula crates.
//!
//! Highlights
//! - Unified `Value` enum with kinds like integers, floats, strings, arrays, objects,
//!   bytes and temporal types (date, time, datetime, duration).
//! - [`ValueKind`] helpers to reason about types (numeric, temporal, collection, etc.).
//! - Clear and composable error types in `core::error`.
//! - Simple validation traits and ready-to-use validators in [`validation`].
//! - Optional `decimal` feature for high-precision numbers.
//!
//! Quick start
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
//! Validation example
//! ```rust
//! use nebula_value::{Value, TextLength, ValueValidationExt};
//!
//! let v = Value::from("hello");
//! v.validate_with(&TextLength::default().min_length(3).max_length(10)).unwrap();
//! ```
//!
//! See the `core` and `validation` modules for more details.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

mod core;
mod types;
mod validation;

pub use core::*;
pub use types::*;
pub use validation::*;
