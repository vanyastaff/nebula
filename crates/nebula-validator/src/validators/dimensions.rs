//! Dimension and mathematical validation operations using the unified validator macro

use crate::{validator, validator_fn};

// ==================== MATHEMATICAL VALIDATORS ====================

validator! {
    /// Validator that checks if number is divisible by another number
    pub struct DivisibleBy {
        divisor: f64
    }
    impl {
        fn check(value: &Value, divisor: &f64) -> bool {
            {
                if let Some(number) = value.as_f64() {
                    if *divisor == 0.0 {
                        false
                    } else {
                        number % divisor == 0.0
                    }
                } else {
                    false
                }
            }
        }
        fn error(divisor: &f64) -> String {
            { format!("Number must be divisible by {}", divisor) }
        }
        const DESCRIPTION: &str = "Number must be divisible by specified divisor";
    }
}

validator! {
    /// Validator that checks if number is even
    pub struct Even {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_i64().map_or(false, |n| n % 2 == 0) }
        }
        fn error() -> String {
            { "Number must be even".to_string() }
        }
        const DESCRIPTION: &str = "Number must be even";
    }
}

validator! {
    /// Validator that checks if number is odd
    pub struct Odd {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.as_i64().map_or(false, |n| n % 2 != 0) }
        }
        fn error() -> String {
            { "Number must be odd".to_string() }
        }
        const DESCRIPTION: &str = "Number must be odd";
    }
}

validator! {
    /// Validator that checks if number is a perfect square
    pub struct PerfectSquare {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if let Some(number) = value.as_f64() {
                    if number < 0.0 {
                        false
                    } else {
                        let sqrt = number.sqrt();
                        (sqrt.floor() - sqrt).abs() < f64::EPSILON
                    }
                } else {
                    false
                }
            }
        }
        fn error() -> String {
            { "Number must be a perfect square".to_string() }
        }
        const DESCRIPTION: &str = "Number must be a perfect square";
    }
}

// ==================== TYPE ALIASES ====================

/// Type alias for DivisibleBy for better readability
pub type MultipleOf = DivisibleBy;

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn divisible_by(divisor: f64) -> DivisibleBy);
validator_fn!(pub fn even() -> Even);
validator_fn!(pub fn odd() -> Odd);
validator_fn!(pub fn perfect_square() -> PerfectSquare);

// Type alias convenience function
pub fn multiple_of(divisor: f64) -> MultipleOf {
    DivisibleBy::new(divisor)
}

// Mathematical convenience functions
pub fn is_multiple_of(divisor: f64) -> DivisibleBy {
    DivisibleBy::new(divisor)
}

pub fn is_even() -> Even {
    Even::new()
}

pub fn is_odd() -> Odd {
    Odd::new()
}

pub fn is_perfect_square() -> PerfectSquare {
    PerfectSquare::new()
}