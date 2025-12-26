//! Number types for nebula-value
//!
//! This module provides scalar numeric types:
//! - `Integer`: i64 with checked arithmetic
//! - `Float`: f64 without Eq (NaN-aware)
//! - `HashableFloat`: Float wrapper with Eq/Hash for collections

/// 64-bit floating point number
pub mod float;
/// Hashable float wrapper for use in collections
pub mod hashable;
/// 64-bit signed integer with checked arithmetic
pub mod integer;

pub use float::Float;
pub use hashable::HashableFloat;
pub use integer::Integer;
