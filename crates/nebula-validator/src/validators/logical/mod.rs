//! Logical validators
//!
//! Validators for boolean logic and nullable values.

pub mod boolean;
pub mod nullable;

// Re-export boolean validators
pub use boolean::{is_false, is_true, IsFalse, IsTrue};

// Re-export nullable validators
pub use nullable::{not_null, required, NotNull, Required};

pub mod prelude {
    pub use super::boolean::*;
    pub use super::nullable::*;
}