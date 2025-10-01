//! Collection validation operations using the unified validator macro

use crate::{validator, validator_fn, ValueExt};

// ==================== SIZE VALIDATORS ====================

validator! {
    /// Validator that checks exact collection size
    pub struct Size {
        size: usize
    }
    impl {
        fn check(value: &Value, size: &usize) -> bool {
            {
                // Use value.len() which works for arrays, objects, strings, and bytes
                if value.is_collection() || value.is_text() {
                    value.len() == *size
                } else {
                    false
                }
            }
        }
        fn error(size: &usize) -> String {
            { format!("Collection size must be exactly {}", size) }
        }
        const DESCRIPTION: &str = "Collection must have exact size";
    }
}

validator! {
    /// Validator that checks minimum collection size
    pub struct MinSize {
        min_size: usize
    }
    impl {
        fn check(value: &Value, min_size: &usize) -> bool {
            {
                if value.is_collection() || value.is_text() {
                    value.len() >= *min_size
                } else {
                    false
                }
            }
        }
        fn error(min_size: &usize) -> String {
            { format!("Collection size must be at least {}", min_size) }
        }
        const DESCRIPTION: &str = "Collection must meet minimum size requirement";
    }
}

validator! {
    /// Validator that checks maximum collection size
    pub struct MaxSize {
        max_size: usize
    }
    impl {
        fn check(value: &Value, max_size: &usize) -> bool {
            {
                if value.is_collection() || value.is_text() {
                    value.len() <= *max_size
                } else {
                    false
                }
            }
        }
        fn error(max_size: &usize) -> String {
            { format!("Collection size must be at most {}", max_size) }
        }
        const DESCRIPTION: &str = "Collection must not exceed maximum size";
    }
}

validator! {
    /// Validator that checks collection size is within a range
    pub struct SizeRange {
        min_size: usize,
        max_size: usize
    }
    impl {
        fn check(value: &Value, min_size: &usize, max_size: &usize) -> bool {
            {
                let collection_size = match value {
                    nebula_value::Value::Array(arr) => Some(arr.len()),
                    nebula_value::Value::Object(obj) => Some(obj.len()),
                    nebula_value::Value::Text(s) => Some(s.len()),
                    _ => None,
                };
                collection_size.map_or(false, |s| s >= *min_size && s <= *max_size)
            }
        }
        fn error(min_size: &usize, max_size: &usize) -> String {
            { format!("Collection size must be between {} and {}", min_size, max_size) }
        }
        const DESCRIPTION: &str = "Collection size must be within specified range";
    }
}

// ==================== ARRAY CONTENT VALIDATORS ====================

validator! {
    /// Validator that checks if array contains a specific value
    pub struct ArrayContains {
        value: nebula_value::Value
    }
    impl {
        fn check(arr: &Value, value: &nebula_value::Value) -> bool {
            {
                // Compare values by string representation (workaround for mixed Value types)
                if let nebula_value::Value::Array(a) = arr {
                    let value_str = value.to_string();
                    return a.iter().any(|item| item.to_string() == value_str);
                }
                false
            }
        }
        fn error(value: &nebula_value::Value) -> String {
            { format!("Array must contain the value: {}", value) }
        }
        const DESCRIPTION: &str = "Array must contain specified value";
    }
}

validator! {
    /// Validator that checks if collection is empty
    pub struct Empty {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if value.is_collection() || value.is_text() {
                    value.is_empty()
                } else {
                    false
                }
            }
        }
        fn error() -> String {
            { "Collection must be empty".to_string() }
        }
        const DESCRIPTION: &str = "Collection must be empty";
    }
}

validator! {
    /// Validator that checks if collection is not empty
    pub struct NotEmpty {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if value.is_collection() || value.is_text() {
                    !value.is_empty()
                } else {
                    false
                }
            }
        }
        fn error() -> String {
            { "Collection must not be empty".to_string() }
        }
        const DESCRIPTION: &str = "Collection must not be empty";
    }
}

validator! {
    /// Validator that checks if array has unique elements
    pub struct Unique {
    }
    impl {
        fn check(value: &Value) -> bool {
            {
                if let nebula_value::Value::Array(arr) = value {
                    let mut seen = std::collections::HashSet::new();
                    for item in arr.iter() {
                        if !seen.insert(item.to_string()) {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }
        }
        fn error() -> String {
            { "Array must contain unique elements".to_string() }
        }
        const DESCRIPTION: &str = "Array elements must be unique";
    }
}

// ==================== CONVENIENCE FUNCTIONS ====================

validator_fn!(pub fn size(size: usize) -> Size);
validator_fn!(pub fn min_size(min_size: usize) -> MinSize);
validator_fn!(pub fn max_size(max_size: usize) -> MaxSize);
validator_fn!(pub fn size_range(min_size: usize, max_size: usize) -> SizeRange);
validator_fn!(pub fn array_contains(value: nebula_value::Value) -> ArrayContains);
validator_fn!(pub fn empty() -> Empty);
validator_fn!(pub fn not_empty() -> NotEmpty);
validator_fn!(pub fn unique() -> Unique);

// Specialized convenience functions
pub fn contains_value(value: nebula_value::Value) -> ArrayContains {
    ArrayContains::new(value)
}

pub fn array_size(size: usize) -> Size {
    Size::new(size)
}

pub fn array_min_size(min_size: usize) -> MinSize {
    MinSize::new(min_size)
}

pub fn array_max_size(max_size: usize) -> MaxSize {
    MaxSize::new(max_size)
}