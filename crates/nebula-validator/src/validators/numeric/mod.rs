//! Numeric validators
//!
//! This module provides validators for numeric types.

pub mod divisibility;
pub mod float;
pub mod percentage;
pub mod properties;
pub mod range;

// Re-export divisibility validators
pub use divisibility::{DivisibleBy, divisible_by, multiple_of};

// Re-export float validators
pub use float::{
    DecimalPlaces, Finite, FiniteF32, NotNaN, NotNaNF32, decimal_places, finite, finite_f32,
    not_nan, not_nan_f32,
};

// Re-export percentage validators
pub use percentage::{
    Percentage, Percentage100, Percentage100F64, PercentageF32, percentage, percentage_100,
    percentage_100_f64, percentage_f32,
};

// Re-export property validators
pub use properties::{
    Even, Negative, NonZero, Odd, Positive, PowerOfTwo, PowerOfTwoU64, even, negative, non_zero,
    odd, positive, power_of_two, power_of_two_u64,
};

// Re-export range validators
pub use range::{
    ExclusiveRange, GreaterThan, InRange, LessThan, Max, Min, exclusive_range, greater_than,
    in_range, less_than, max, min,
};

/// Prelude for numeric validators.
pub mod prelude {
    pub use super::divisibility::*;
    pub use super::float::*;
    pub use super::percentage::*;
    pub use super::properties::*;
    pub use super::range::*;
}
