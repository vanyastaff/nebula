//! Structural validation operations using the unified validator macro

use crate::{validator, validator_fn, ValueExt};

// ==================== STRUCTURAL VALIDATORS ====================

validator! {
    /// Validator that checks if object has a specific key
    pub struct HasKey {
        key: String
    }
    impl {
        fn check(value: &Value, key: &String) -> bool {
            {
                value.contains_key(key)
            }
        }
        fn error(key: &String) -> String {
            { format!("Object must have key '{}'", key) }
        }
        const DESCRIPTION: &str = "Object must have specified key";
    }
}

validator! {
    /// Validator that checks if object has all specified keys
    pub struct HasAllKeys {
        keys: Vec<String>
    }
    impl {
        fn check(value: &Value, keys: &Vec<String>) -> bool {
            {
                if value.is_object() {
                    keys.iter().all(|key| value.contains_key(key))
                } else {
                    false
                }
            }
        }
        fn error(keys: &Vec<String>) -> String {
            { format!("Object must have all keys: {:?}", keys) }
        }
        const DESCRIPTION: &str = "Object must have all specified keys";
    }
}

validator! {
    /// Validator that checks if object has any of the specified keys
    pub struct HasAnyKey {
        keys: Vec<String>
    }
    impl {
        fn check(value: &Value, keys: &Vec<String>) -> bool {
            {
                if value.is_object() {
                    keys.iter().any(|key| value.contains_key(key))
                } else {
                    false
                }
            }
        }
        fn error(keys: &Vec<String>) -> String {
            { format!("Object must have at least one of keys: {:?}", keys) }
        }
        const DESCRIPTION: &str = "Object must have at least one of the specified keys";
    }
}

validator! {
    /// Validator that checks if object has only allowed keys
    pub struct OnlyKeys {
        allowed_keys: Vec<String>
    }
    impl {
        fn check(value: &Value, allowed_keys: &Vec<String>) -> bool {
            {
                if let Some(object) = value.as_object() {
                    object.keys().all(|key| allowed_keys.contains(key))
                } else {
                    false
                }
            }
        }
        fn error(allowed_keys: &Vec<String>) -> String {
            { format!("Object must only have keys: {:?}", allowed_keys) }
        }
        const DESCRIPTION: &str = "Object must only have allowed keys";
    }
}

validator! {
    /// Validator that checks if array has specific length
    pub struct ArrayLength {
        length: usize
    }
    impl {
        fn check(value: &Value, length: &usize) -> bool {
            {
                if let Some(array) = value.as_array() {
                    array.len() == *length
                } else {
                    false
                }
            }
        }
        fn error(length: &usize) -> String {
            { format!("Array must have exactly {} elements", length) }
        }
        const DESCRIPTION: &str = "Array must have exact length";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn has_key(key: String) -> HasKey);
validator_fn!(pub fn has_all_keys(keys: Vec<String>) -> HasAllKeys);
validator_fn!(pub fn has_any_key(keys: Vec<String>) -> HasAnyKey);
validator_fn!(pub fn only_keys(allowed_keys: Vec<String>) -> OnlyKeys);
validator_fn!(pub fn array_length(length: usize) -> ArrayLength);

// String-specific convenience functions with &str input
pub fn object_has_key(key: &str) -> HasKey {
    HasKey::new(key.to_string())
}

pub fn required_keys(keys: Vec<&str>) -> HasAllKeys {
    HasAllKeys::new(keys.into_iter().map(|s| s.to_string()).collect())
}

pub fn optional_keys(keys: Vec<&str>) -> HasAnyKey {
    HasAnyKey::new(keys.into_iter().map(|s| s.to_string()).collect())
}

pub fn allowed_keys(keys: Vec<&str>) -> OnlyKeys {
    OnlyKeys::new(keys.into_iter().map(|s| s.to_string()).collect())
}

pub fn exact_array_length(length: usize) -> ArrayLength {
    ArrayLength::new(length)
}