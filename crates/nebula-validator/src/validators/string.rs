//! String validation operations using the unified validator macro
//!
//! This module provides comprehensive string validators including length checks,
//! content validation, character type validation, and format verification.

use crate::{validator, validator_fn};
use bon::builder;

// ==================== STRING LENGTH VALIDATORS ====================

validator! {
    /// Validator that checks string length exactly
    pub struct StringLength {
        length: usize
    }
    impl {
        fn check(value: &Value, length: &usize) -> bool {
            { value.as_str().map_or(false, |s| s.len() == *length) }
        }
        fn error(length: &usize) -> String {
            { format!("String must be exactly {} characters long", length) }
        }
        const DESCRIPTION: &str = "String must have exact length";
    }
}

validator! {
    /// Validator that checks minimum string length
    pub struct MinLength {
        min_length: usize
    }
    impl {
        fn check(value: &Value, min_length: &usize) -> bool {
            { value.as_str().map_or(false, |s| s.len() >= *min_length) }
        }
        fn error(min_length: &usize) -> String {
            { format!("String must be at least {} characters long", min_length) }
        }
        const DESCRIPTION: &str = "String must meet minimum length requirement";
    }
}

validator! {
    /// Validator that checks maximum string length
    pub struct MaxLength {
        max_length: usize
    }
    impl {
        fn check(value: &Value, max_length: &usize) -> bool {
            { value.as_str().map_or(false, |s| s.len() <= *max_length) }
        }
        fn error(max_length: &usize) -> String {
            { format!("String must be at most {} characters long", max_length) }
        }
        const DESCRIPTION: &str = "String must not exceed maximum length";
    }
}

validator! {
    /// Validator that checks string length is within a range
    pub struct LengthRange {
        min_length: usize,
        max_length: usize
    }
    impl {
        fn check(value: &Value, min_length: &usize, max_length: &usize) -> bool {
            {
                value.as_str().map_or(false, |s| {
                    let len = s.len();
                    len >= *min_length && len <= *max_length
                })
            }
        }
        fn error(min_length: &usize, max_length: &usize) -> String {
            { format!("String length must be between {} and {} characters", min_length, max_length) }
        }
        const DESCRIPTION: &str = "String length must be within specified range";
    }
}

// ==================== STRING CONTENT VALIDATORS ====================

validator! {
    /// Validator that checks if string contains a substring
    pub struct Contains {
        substring: String
    }
    impl {
        fn check(value: &Value, substring: &String) -> bool {
            { value.as_str().map_or(false, |s| s.contains(substring)) }
        }
        fn error(substring: &String) -> String {
            { format!("String must contain '{}'", substring) }
        }
        const DESCRIPTION: &str = "String must contain specified substring";
    }
}

validator! {
    /// Validator that checks if string starts with a prefix
    pub struct StartsWith {
        prefix: String
    }
    impl {
        fn check(value: &Value, prefix: &String) -> bool {
            { value.as_str().map_or(false, |s| s.starts_with(prefix)) }
        }
        fn error(prefix: &String) -> String {
            { format!("String must start with '{}'", prefix) }
        }
        const DESCRIPTION: &str = "String must start with specified prefix";
    }
}

validator! {
    /// Validator that checks if string ends with a suffix
    pub struct EndsWith {
        suffix: String
    }
    impl {
        fn check(value: &Value, suffix: &String) -> bool {
            { value.as_str().map_or(false, |s| s.ends_with(suffix)) }
        }
        fn error(suffix: &String) -> String {
            { format!("String must end with '{}'", suffix) }
        }
        const DESCRIPTION: &str = "String must end with specified suffix";
    }
}

// ==================== CHARACTER TYPE VALIDATORS ====================

validator! {
    /// Validator that checks if string contains only alphanumeric characters
    pub struct Alphanumeric {
        allow_spaces: bool
    }
    impl {
        fn check(value: &Value, allow_spaces: &bool) -> bool {
            {
                if let Some(string_val) = value.as_str() {
                    if *allow_spaces {
                        string_val.chars().all(|c| c.is_alphanumeric() || c.is_whitespace())
                    } else {
                        string_val.chars().all(|c| c.is_alphanumeric())
                    }
                } else {
                    false
                }
            }
        }
        fn error(allow_spaces: &bool) -> String {
            {
                if *allow_spaces {
                    "String must contain only alphanumeric characters and spaces".to_string()
                } else {
                    "String must contain only alphanumeric characters".to_string()
                }
            }
        }
        const DESCRIPTION: &str = "String must contain only alphanumeric characters";
    }
}

validator! {
    /// Validator that checks if string contains only alphabetic characters
    pub struct Alpha {
        allow_spaces: bool
    }
    impl {
        fn check(value: &Value, allow_spaces: &bool) -> bool {
            {
                if let Some(string_val) = value.as_str() {
                    if *allow_spaces {
                        string_val.chars().all(|c| c.is_alphabetic() || c.is_whitespace())
                    } else {
                        string_val.chars().all(|c| c.is_alphabetic())
                    }
                } else {
                    false
                }
            }
        }
        fn error(allow_spaces: &bool) -> String {
            {
                if *allow_spaces {
                    "String must contain only alphabetic characters and spaces".to_string()
                } else {
                    "String must contain only alphabetic characters".to_string()
                }
            }
        }
        const DESCRIPTION: &str = "String must contain only alphabetic characters";
    }
}

validator! {
    /// Validator that checks if string contains only numeric characters
    pub struct NumericString {
        allow_decimal: bool,
        allow_negative: bool
    }
    impl {
        fn check(value: &Value, allow_decimal: &bool, allow_negative: &bool) -> bool {
            {
                if let Some(string_val) = value.as_str() {
                    if string_val.is_empty() {
                        return false;
                    }

                    let mut chars = string_val.chars();
                    let mut has_decimal = false;

                    // Check for negative sign at the start
                    if let Some(first_char) = chars.next() {
                        if first_char == '-' {
                            if !*allow_negative {
                                return false;
                            }
                        } else if first_char == '.' {
                            if !*allow_decimal {
                                return false;
                            }
                            has_decimal = true;
                        } else if !first_char.is_ascii_digit() {
                            return false;
                        }
                    }

                    // Check remaining characters
                    for c in chars {
                        if c == '.' {
                            if !*allow_decimal || has_decimal {
                                return false;
                            }
                            has_decimal = true;
                        } else if !c.is_ascii_digit() {
                            return false;
                        }
                    }

                    true
                } else {
                    false
                }
            }
        }
        fn error(allow_decimal: &bool, allow_negative: &bool) -> String {
            {
                match (*allow_decimal, *allow_negative) {
                    (true, true) => "String must be a valid signed decimal number".to_string(),
                    (true, false) => "String must be a valid decimal number".to_string(),
                    (false, true) => "String must be a valid signed integer".to_string(),
                    (false, false) => "String must contain only numeric characters".to_string(),
                }
            }
        }
        const DESCRIPTION: &str = "String must be numeric";
    }
}

// ==================== CASE VALIDATORS ====================

validator! {
    /// Validator that checks if string is all uppercase
    pub struct Uppercase {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if let Some(string_val) = value.as_str() {
                    string_val == string_val.to_uppercase()
                } else {
                    false
                }
            }
        }
        fn error() -> String {
            { "String must be uppercase".to_string() }
        }
        const DESCRIPTION: &str = "String must be uppercase";
    }
}

validator! {
    /// Validator that checks if string is all lowercase
    pub struct Lowercase {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if let Some(string_val) = value.as_str() {
                    string_val == string_val.to_lowercase()
                } else {
                    false
                }
            }
        }
        fn error() -> String {
            { "String must be lowercase".to_string() }
        }
        const DESCRIPTION: &str = "String must be lowercase";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

// Basic string validators
validator_fn!(pub fn string_length(length: usize) -> StringLength);
validator_fn!(pub fn min_length(min_length: usize) -> MinLength);
validator_fn!(pub fn max_length(max_length: usize) -> MaxLength);
validator_fn!(pub fn length_range(min_length: usize, max_length: usize) -> LengthRange);

// Content validators
validator_fn!(pub fn string_contains(substring: String) -> Contains);
validator_fn!(pub fn string_starts_with(prefix: String) -> StartsWith);
validator_fn!(pub fn string_ends_with(suffix: String) -> EndsWith);

// Character type validators
validator_fn!(pub fn alphanumeric(allow_spaces: bool) -> Alphanumeric);
validator_fn!(pub fn alpha(allow_spaces: bool) -> Alpha);
validator_fn!(pub fn numeric_string(allow_decimal: bool, allow_negative: bool) -> NumericString);

// Case validators
validator_fn!(pub fn uppercase() -> Uppercase);
validator_fn!(pub fn lowercase() -> Lowercase);

// String-specific convenience functions with &str input
pub fn contains_str(substring: &str) -> Contains {
    Contains::new(substring.to_string())
}

pub fn starts_with_str(prefix: &str) -> StartsWith {
    StartsWith::new(prefix.to_string())
}

pub fn ends_with_str(suffix: &str) -> EndsWith {
    EndsWith::new(suffix.to_string())
}

// Character type convenience functions with default parameters
pub fn alphanumeric_only() -> Alphanumeric {
    Alphanumeric::new(false)
}

pub fn alphanumeric_with_spaces() -> Alphanumeric {
    Alphanumeric::new(true)
}

pub fn alpha_only() -> Alpha {
    Alpha::new(false)
}

pub fn alpha_with_spaces() -> Alpha {
    Alpha::new(true)
}

pub fn numeric_only() -> NumericString {
    NumericString::new(false, false)
}

pub fn decimal_string() -> NumericString {
    NumericString::new(true, false)
}

pub fn signed_numeric() -> NumericString {
    NumericString::new(false, true)
}

pub fn signed_decimal() -> NumericString {
    NumericString::new(true, true)
}

// ==================== BUILDER-BASED API ====================

/// Create alphanumeric validator with builder pattern
#[builder]
pub fn alphanumeric_builder(#[builder(default = false)] allow_spaces: bool) -> Alphanumeric {
    Alphanumeric::new(allow_spaces)
}

/// Create alphabetic validator with builder pattern
#[builder]
pub fn alpha_builder(#[builder(default = false)] allow_spaces: bool) -> Alpha {
    Alpha::new(allow_spaces)
}

/// Create numeric string validator with builder pattern
#[builder]
pub fn numeric_string_builder(
    #[builder(default = false)] allow_decimal: bool,
    #[builder(default = false)] allow_negative: bool,
) -> NumericString {
    NumericString::new(allow_decimal, allow_negative)
}
