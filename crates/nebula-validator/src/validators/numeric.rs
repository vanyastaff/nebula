//! Numeric validation operations using the unified validator macro

use crate::{ValueExt, validator, validator_fn};
use bon::builder;

// ==================== NUMERIC RANGE VALIDATORS ====================

validator! {
    /// Validator that checks minimum numeric value
    pub struct Min {
        min_value: f64
    }
    impl {
        fn check(value: &Value, min_value: &f64) -> bool {
            { value.as_f64().map_or(false, |v| v >= *min_value) }
        }
        fn error(min_value: &f64) -> String {
            { format!("Value must be at least {}", min_value) }
        }
        const DESCRIPTION: &str = "Number must meet minimum value requirement";
    }
}

validator! {
    /// Validator that checks maximum numeric value
    pub struct Max {
        max_value: f64
    }
    impl {
        fn check(value: &Value, max_value: &f64) -> bool {
            { value.as_f64().map_or(false, |v| v <= *max_value) }
        }
        fn error(max_value: &f64) -> String {
            { format!("Value must be at most {}", max_value) }
        }
        const DESCRIPTION: &str = "Number must not exceed maximum value";
    }
}

validator! {
    /// Validator that checks numeric value is within a range
    pub struct Range {
        min_value: f64,
        max_value: f64
    }
    impl {
        fn check(value: &Value, min_value: &f64, max_value: &f64) -> bool {
            { value.as_f64().map_or(false, |v| v >= *min_value && v <= *max_value) }
        }
        fn error(min_value: &f64, max_value: &f64) -> String {
            { format!("Value must be between {} and {}", min_value, max_value) }
        }
        const DESCRIPTION: &str = "Number must be within specified range";
    }
}

// ==================== SIGN VALIDATORS ====================

validator! {
    /// Validator that checks if number is positive (> 0)
    pub struct Positive {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_f64().map_or(false, |v| v > 0.0) }
        }
        fn error() -> String {
            { "Value must be positive".to_string() }
        }
        const DESCRIPTION: &str = "Number must be greater than zero";
    }
}

validator! {
    /// Validator that checks if number is negative (< 0)
    pub struct Negative {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_f64().map_or(false, |v| v < 0.0) }
        }
        fn error() -> String {
            { "Value must be negative".to_string() }
        }
        const DESCRIPTION: &str = "Number must be less than zero";
    }
}

validator! {
    /// Validator that checks if number is zero
    pub struct Zero {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_f64().map_or(false, |v| v == 0.0) }
        }
        fn error() -> String {
            { "Value must be zero".to_string() }
        }
        const DESCRIPTION: &str = "Number must be exactly zero";
    }
}

validator! {
    /// Validator that checks if number is non-negative (>= 0)
    pub struct NonNegative {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_f64().map_or(false, |v| v >= 0.0) }
        }
        fn error() -> String {
            { "Value must be non-negative".to_string() }
        }
        const DESCRIPTION: &str = "Number must be greater than or equal to zero";
    }
}

validator! {
    /// Validator that checks if number is non-positive (<= 0)
    pub struct NonPositive {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_f64().map_or(false, |v| v <= 0.0) }
        }
        fn error() -> String {
            { "Value must be non-positive".to_string() }
        }
        const DESCRIPTION: &str = "Number must be less than or equal to zero";
    }
}

// ==================== TYPE VALIDATORS ====================

validator! {
    /// Validator that checks if number is an integer
    pub struct Integer {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_f64().map_or(false, |v| v.fract() == 0.0) }
        }
        fn error() -> String {
            { "Value must be an integer".to_string() }
        }
        const DESCRIPTION: &str = "Number must be a whole number";
    }
}

validator! {
    /// Validator that checks if number is finite (not infinity or NaN)
    pub struct Finite {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_f64().map_or(false, |v| v.is_finite()) }
        }
        fn error() -> String {
            { "Value must be finite".to_string() }
        }
        const DESCRIPTION: &str = "Number must be finite (not infinity or NaN)";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn min(min_value: f64) -> Min);
validator_fn!(pub fn max(max_value: f64) -> Max);
validator_fn!(pub fn numeric_range(min_value: f64, max_value: f64) -> Range);
validator_fn!(pub fn positive() -> Positive);
validator_fn!(pub fn negative() -> Negative);
validator_fn!(pub fn zero() -> Zero);
validator_fn!(pub fn non_negative() -> NonNegative);
validator_fn!(pub fn non_positive() -> NonPositive);
validator_fn!(pub fn integer() -> Integer);
validator_fn!(pub fn finite() -> Finite);

// Compatibility aliases
pub fn range(min_value: f64, max_value: f64) -> Range {
    Range::new(min_value, max_value)
}

pub fn between(min_value: f64, max_value: f64) -> Range {
    Range::new(min_value, max_value)
}

// ==================== BUILDER-BASED API ====================

/// Create range validator with builder pattern
#[builder]
pub fn range_builder(min_value: f64, max_value: f64) -> Range {
    Range::new(min_value, max_value)
}

/// Create range validator with optional bounds using builder pattern
#[builder]
pub fn flexible_range(
    #[builder(default = f64::NEG_INFINITY)] min_value: f64,
    #[builder(default = f64::INFINITY)] max_value: f64,
) -> Range {
    Range::new(min_value, max_value)
}
