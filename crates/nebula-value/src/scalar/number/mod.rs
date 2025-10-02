//! Number types for nebula-value
//!
//! This module provides scalar numeric types:
//! - `Integer`: i64 with checked arithmetic
//! - `Float`: f64 without Eq (NaN-aware)
//! - `HashableFloat`: Float wrapper with Eq/Hash for collections

pub mod integer;
pub mod float;
pub mod hashable;

pub use integer::Integer;
pub use float::Float;
pub use hashable::HashableFloat;


