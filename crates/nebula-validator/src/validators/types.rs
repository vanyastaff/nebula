//! Type validation for JSON values using the unified validator macro

use crate::{validator, validator_fn, ValueExt};

// ==================== TYPE VALIDATORS ====================

validator! {
    /// Validator that checks if value is a string
    pub struct IsString {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.is_text() }
        }
        fn error() -> String {
            { "Value must be a string".to_string() }
        }
        const DESCRIPTION: &str = "Value must be a string";
    }
}

validator! {
    /// Validator that checks if value is a number
    pub struct IsNumber {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.is_number() }
        }
        fn error() -> String {
            { "Value must be a number".to_string() }
        }
        const DESCRIPTION: &str = "Value must be a number";
    }
}

validator! {
    /// Validator that checks if value is a boolean
    pub struct IsBoolean {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.is_boolean() }
        }
        fn error() -> String {
            { "Value must be a boolean".to_string() }
        }
        const DESCRIPTION: &str = "Value must be a boolean";
    }
}

validator! {
    /// Validator that checks if value is an array
    pub struct IsArray {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.is_array() }
        }
        fn error() -> String {
            { "Value must be an array".to_string() }
        }
        const DESCRIPTION: &str = "Value must be an array";
    }
}

validator! {
    /// Validator that checks if value is an object
    pub struct IsObject {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.is_object() }
        }
        fn error() -> String {
            { "Value must be an object".to_string() }
        }
        const DESCRIPTION: &str = "Value must be an object";
    }
}

validator! {
    /// Validator that checks if value is null
    pub struct IsNull {
    }
    impl {
        fn check(value: &Value) -> bool {
            { value.is_null() }
        }
        fn error() -> String {
            { "Value must be null".to_string() }
        }
        const DESCRIPTION: &str = "Value must be null";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn is_string() -> IsString);
validator_fn!(pub fn is_number() -> IsNumber);
validator_fn!(pub fn is_boolean() -> IsBoolean);
validator_fn!(pub fn is_array() -> IsArray);
validator_fn!(pub fn is_object() -> IsObject);
validator_fn!(pub fn is_null() -> IsNull);

// Compatibility aliases without "is_" prefix
pub fn string() -> IsString {
    IsString::new()
}

pub fn number() -> IsNumber {
    IsNumber::new()
}

pub fn boolean() -> IsBoolean {
    IsBoolean::new()
}

pub fn array() -> IsArray {
    IsArray::new()
}

pub fn object() -> IsObject {
    IsObject::new()
}