//! Comparison validation operations using the unified validator macro

use crate::{validator, validator_fn};

// ==================== COMPARISON VALIDATORS ====================

validator! {
    /// Validator that checks if value equals another value
    pub struct Equals {
        expected: serde_json::Value
    }
    impl {
        fn check(value: &Value, expected: &serde_json::Value) -> bool {
            { value == expected }
        }
        fn error(expected: &serde_json::Value) -> String {
            { format!("Value must equal {}", expected) }
        }
        const DESCRIPTION: &str = "Value must equal expected value";
    }
}

validator! {
    /// Validator that checks if a value does not equal another value
    pub struct NotEquals {
        forbidden: serde_json::Value
    }
    impl {
        fn check(value: &Value, forbidden: &serde_json::Value) -> bool {
            { value != forbidden }
        }
        fn error(forbidden: &serde_json::Value) -> String {
            { format!("Value must not equal {}", forbidden) }
        }
        const DESCRIPTION: &str = "Value must not equal forbidden value";
    }
}

validator! {
    /// Validator that checks if numeric value is greater than threshold
    pub struct GreaterThan {
        threshold: f64
    }
    impl {
        fn check(value: &Value, threshold: &f64) -> bool {
            { value.as_f64().map_or(false, |v| v > *threshold) }
        }
        fn error(threshold: &f64) -> String {
            { format!("Value must be greater than {}", threshold) }
        }
        const DESCRIPTION: &str = "Value must be greater than threshold";
    }
}

validator! {
    /// Validator that checks if numeric value is greater than or equal to threshold
    pub struct GreaterThanOrEqual {
        threshold: f64
    }
    impl {
        fn check(value: &Value, threshold: &f64) -> bool {
            { value.as_f64().map_or(false, |v| v >= *threshold) }
        }
        fn error(threshold: &f64) -> String {
            { format!("Value must be greater than or equal to {}", threshold) }
        }
        const DESCRIPTION: &str = "Value must be greater than or equal to threshold";
    }
}

validator! {
    /// Validator that checks if numeric value is less than threshold
    pub struct LessThan {
        threshold: f64
    }
    impl {
        fn check(value: &Value, threshold: &f64) -> bool {
            { value.as_f64().map_or(false, |v| v < *threshold) }
        }
        fn error(threshold: &f64) -> String {
            { format!("Value must be less than {}", threshold) }
        }
        const DESCRIPTION: &str = "Value must be less than threshold";
    }
}

validator! {
    /// Validator that checks if numeric value is less than or equal to threshold
    pub struct LessThanOrEqual {
        threshold: f64
    }
    impl {
        fn check(value: &Value, threshold: &f64) -> bool {
            { value.as_f64().map_or(false, |v| v <= *threshold) }
        }
        fn error(threshold: &f64) -> String {
            { format!("Value must be less than or equal to {}", threshold) }
        }
        const DESCRIPTION: &str = "Value must be less than or equal to threshold";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn equals(expected: serde_json::Value) -> Equals);
validator_fn!(pub fn not_equals(forbidden: serde_json::Value) -> NotEquals);
validator_fn!(pub fn greater_than(threshold: f64) -> GreaterThan);
validator_fn!(pub fn greater_than_or_equal(threshold: f64) -> GreaterThanOrEqual);
validator_fn!(pub fn less_than(threshold: f64) -> LessThan);
validator_fn!(pub fn less_than_or_equal(threshold: f64) -> LessThanOrEqual);

// Simplified names
pub fn gt(threshold: f64) -> GreaterThan {
    GreaterThan::new(threshold)
}

pub fn gte(threshold: f64) -> GreaterThanOrEqual {
    GreaterThanOrEqual::new(threshold)
}

pub fn lt(threshold: f64) -> LessThan {
    LessThan::new(threshold)
}

pub fn lte(threshold: f64) -> LessThanOrEqual {
    LessThanOrEqual::new(threshold)
}

pub fn eq(expected: serde_json::Value) -> Equals {
    Equals::new(expected)
}

pub fn ne(forbidden: serde_json::Value) -> NotEquals {
    NotEquals::new(forbidden)
}