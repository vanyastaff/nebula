// Core modules
pub mod error;
pub mod kind;
pub mod path;
pub mod value;

// Re-exports for convenience
pub use error::{ValueError, ValueResult};
pub use kind::{ValueKind, TypeCompatibility};
pub use path::{PathSegment, ValuePath};
pub use value::Value;

// Optional: Create type aliases for common use cases
pub type DynResult<T> = Result<T, Box<dyn std::error::Error>>;

// Prelude module for easy imports
pub mod prelude {
    pub use super::{
        Value,
        ValueError,
        ValueResult,
        ValueKind,
        ValuePath,
        PathSegment,
    };

    // Re-export types from types module
    pub use crate::types::*;
}