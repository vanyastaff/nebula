//! Numeric validators
//!
//! This module provides validators for numeric types.

pub mod properties;
pub mod range;

// Re-export range validators
pub use range::{InRange, Max, Min, in_range, max, min};

// Re-export property validators
pub use properties::{Even, Negative, Odd, Positive, even, negative, odd, positive};

/// Prelude for numeric validators.
pub mod prelude {
    pub use super::properties::*;
    pub use super::range::*;
}
