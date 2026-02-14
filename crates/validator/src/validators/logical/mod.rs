//! Logical validators
//!
//! Validators for boolean logic and nullable values.

pub mod boolean;
pub mod nullable;

// Re-export boolean validators
pub use boolean::{IsFalse, IsTrue, is_false, is_true};

// Re-export nullable validators
pub use nullable::{NotNull, Required, not_null, required};

pub mod prelude {
    pub use super::boolean::*;
    pub use super::nullable::*;
}
