//! Scalar types for nebula-value
//!
//! This module contains scalar (non-collection) value types.

pub mod number;
pub mod text;
pub mod bytes;

// Re-exports
pub use number::{Integer, Float, HashableFloat};
pub use text::Text;
pub use bytes::Bytes;
