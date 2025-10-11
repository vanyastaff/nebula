//! Collection types for nebula-value
//!
//! This module provides collection types with persistent data structures:
//! - Array: Ordered sequence (`im::Vector`)
//! - Object: Key-value map (`im::HashMap`)

pub mod array;
pub mod object;

// Re-exports
pub use array::Array;
pub use object::Object;
