//! String pattern validation operations using the unified validator macro

use crate::{validator, validator_fn};

// ==================== PATTERN VALIDATORS ====================

validator! {
    /// Validator that checks if string starts with a specific prefix
    pub struct PatternStartsWith {
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
    /// Validator that checks if string ends with a specific suffix
    pub struct PatternEndsWith {
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

validator! {
    /// Validator that checks if string contains a specific pattern
    pub struct PatternContains {
        substring: String
    }
    impl {
        fn check(value: &Value, substring: &String) -> bool {
            { value.as_str().map_or(false, |s| s.contains(substring)) }
        }
        fn error(substring: &String) -> String {
            { format!("String must contain '{}'", substring) }
        }
        const DESCRIPTION: &str = "String must contain specified pattern";
    }
}

validator! {
    /// Validator that checks if string does not contain a specific pattern
    pub struct PatternNotContains {
        forbidden: String
    }
    impl {
        fn check(value: &Value, forbidden: &String) -> bool {
            { value.as_str().map_or(false, |s| !s.contains(forbidden)) }
        }
        fn error(forbidden: &String) -> String {
            { format!("String must not contain '{}'", forbidden) }
        }
        const DESCRIPTION: &str = "String must not contain forbidden pattern";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn pattern_starts_with(prefix: String) -> PatternStartsWith);
validator_fn!(pub fn pattern_ends_with(suffix: String) -> PatternEndsWith);
validator_fn!(pub fn pattern_contains(substring: String) -> PatternContains);
validator_fn!(pub fn pattern_not_contains(forbidden: String) -> PatternNotContains);

// String-specific convenience functions with &str input
pub fn starts_with_pattern(prefix: &str) -> PatternStartsWith {
    PatternStartsWith::new(prefix.to_string())
}

pub fn ends_with_pattern(suffix: &str) -> PatternEndsWith {
    PatternEndsWith::new(suffix.to_string())
}

pub fn contains_pattern(substring: &str) -> PatternContains {
    PatternContains::new(substring.to_string())
}

pub fn not_contains_pattern(forbidden: &str) -> PatternNotContains {
    PatternNotContains::new(forbidden.to_string())
}