//! Number types for nebula-value
//!
//! This module provides scalar numeric types:
//! - `Integer`: i64 with checked arithmetic
//! - `Float`: f64 without Eq (NaN-aware)
//! - `HashableFloat`: Float wrapper with Eq/Hash for collections

pub mod float;
pub mod hashable;
pub mod integer;

pub use float::Float;
pub use hashable::HashableFloat;
pub use integer::Integer;
