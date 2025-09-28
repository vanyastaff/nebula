//! Set-based validation operations using the unified validator macro

use crate::{validator, validator_fn};

// ==================== SET VALIDATORS ====================

validator! {
    /// Validator that checks if value is in a set of allowed values
    pub struct In {
        allowed_values: Vec<serde_json::Value>
    }
    impl {
        fn check(value: &Value, allowed_values: &Vec<serde_json::Value>) -> bool {
            { allowed_values.contains(value) }
        }
        fn error(allowed_values: &Vec<serde_json::Value>) -> String {
            { format!("Value must be one of the allowed values: {:?}", allowed_values) }
        }
        const DESCRIPTION: &str = "Value must be in the allowed set";
    }
}

validator! {
    /// Validator that checks if value is NOT in a set of forbidden values
    pub struct NotIn {
        forbidden_values: Vec<serde_json::Value>
    }
    impl {
        fn check(value: &Value, forbidden_values: &Vec<serde_json::Value>) -> bool {
            { !forbidden_values.contains(value) }
        }
        fn error(forbidden_values: &Vec<serde_json::Value>) -> String {
            { format!("Value must not be one of the forbidden values: {:?}", forbidden_values) }
        }
        const DESCRIPTION: &str = "Value must not be in the forbidden set";
    }
}

validator! {
    /// Validator that checks if string value is in a set of allowed strings
    pub struct InStrings {
        allowed_strings: Vec<String>
    }
    impl {
        fn check(value: &Value, allowed_strings: &Vec<String>) -> bool {
            {
                if let Some(s) = value.as_str() {
                    allowed_strings.iter().any(|allowed| allowed == s)
                } else {
                    false
                }
            }
        }
        fn error(allowed_strings: &Vec<String>) -> String {
            { format!("String must be one of: {:?}", allowed_strings) }
        }
        const DESCRIPTION: &str = "String must be in the allowed set";
    }
}

validator! {
    /// Validator that checks if string value is NOT in a set of forbidden strings
    pub struct NotInStrings {
        forbidden_strings: Vec<String>
    }
    impl {
        fn check(value: &Value, forbidden_strings: &Vec<String>) -> bool {
            {
                if let Some(s) = value.as_str() {
                    !forbidden_strings.iter().any(|forbidden| forbidden == s)
                } else {
                    true  // Non-strings are allowed
                }
            }
        }
        fn error(forbidden_strings: &Vec<String>) -> String {
            { format!("String must not be one of: {:?}", forbidden_strings) }
        }
        const DESCRIPTION: &str = "String must not be in the forbidden set";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn in_values(allowed_values: Vec<serde_json::Value>) -> In);
validator_fn!(pub fn not_in_values(forbidden_values: Vec<serde_json::Value>) -> NotIn);
validator_fn!(pub fn in_strings(allowed_strings: Vec<String>) -> InStrings);
validator_fn!(pub fn not_in_strings(forbidden_strings: Vec<String>) -> NotInStrings);

// String-specific convenience functions with &str input
pub fn in_str_values(options: Vec<&str>) -> InStrings {
    let values: Vec<String> = options.into_iter().map(|s| s.to_string()).collect();
    InStrings::new(values)
}

pub fn not_in_str_values(forbidden: Vec<&str>) -> NotInStrings {
    let values: Vec<String> = forbidden.into_iter().map(|s| s.to_string()).collect();
    NotInStrings::new(values)
}

// JSON value convenience functions
pub fn one_of(values: Vec<serde_json::Value>) -> In {
    In::new(values)
}

pub fn none_of(values: Vec<serde_json::Value>) -> NotIn {
    NotIn::new(values)
}