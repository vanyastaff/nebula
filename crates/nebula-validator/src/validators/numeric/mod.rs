//! Numeric validators
//!
//! This module provides validators for numeric types.

pub mod properties;
pub mod range;

// Re-export range validators
pub use range::{in_range, max, min, InRange, Max, Min};

// Re-export property validators
pub use properties::{even, negative, odd, positive, Even, Negative, Odd, Positive};

/// Prelude for numeric validators.
pub mod prelude {
    pub use super::properties::*;
    pub use super::range::*;
}