//! Basic validation operations using the unified validator macro

use crate::{validator, validator_fn, ValueExt};

// ==================== BASIC VALIDATORS ====================

validator! {
    /// Validator that ensures a value is not null
    pub struct NotNull {
    }
    impl {
        fn check(value: &Value) -> bool {
            { !value.is_null() }
        }
        fn error() -> String {
            { "Value cannot be null".to_string() }
        }
        const DESCRIPTION: &str = "Value must not be null";
    }
}

validator! {
    /// Validator that ensures a value is not empty/null/missing
    pub struct Required {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if value.is_null() {
                    false
                } else if value.is_collection() || value.is_text() {
                    !value.is_empty()
                } else {
                    true
                }
            }
        }
        fn error() -> String {
            { "Value is required and cannot be empty".to_string() }
        }
        const DESCRIPTION: &str = "Value must be present and not empty";
    }
}

validator! {
    /// Validator that ensures a value is defined (not null or undefined)
    pub struct Defined {
    }
    impl {
        fn check(value: &Value) -> bool {
            { !value.is_null() }
        }
        fn error() -> String {
            { "Value must be defined".to_string() }
        }
        const DESCRIPTION: &str = "Value must be defined (not null)";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn not_null() -> NotNull);
validator_fn!(pub fn required() -> Required);
validator_fn!(pub fn defined() -> Defined);