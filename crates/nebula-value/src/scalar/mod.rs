//! Scalar types for nebula-value
//!
//! This module contains scalar (non-collection) value types.

pub mod bytes;
pub mod number;
pub mod text;

// Re-exports
pub use bytes::Bytes;
pub use number::{Float, HashableFloat, Integer};
pub use text::Text;
