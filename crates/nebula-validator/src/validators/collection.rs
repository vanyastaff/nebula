//! Collection validation operations using the unified validator macro

use crate::{validator, validator_fn};

// ==================== SIZE VALIDATORS ====================

validator! {
    /// Validator that checks exact collection size
    pub struct Size {
        size: usize
    }
    impl {
        fn check(value: &Value, size: &usize) -> bool {
            {
                let collection_size = match value {
                    serde_json::Value::Array(arr) => Some(arr.len()),
                    serde_json::Value::Object(obj) => Some(obj.len()),
                    serde_json::Value::String(s) => Some(s.len()),
                    _ => None,
                };
                collection_size.map_or(false, |s| s == *size)
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
                let collection_size = match value {
                    serde_json::Value::Array(arr) => Some(arr.len()),
                    serde_json::Value::Object(obj) => Some(obj.len()),
                    serde_json::Value::String(s) => Some(s.len()),
                    _ => None,
                };
                collection_size.map_or(false, |s| s >= *min_size)
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
                let collection_size = match value {
                    serde_json::Value::Array(arr) => Some(arr.len()),
                    serde_json::Value::Object(obj) => Some(obj.len()),
                    serde_json::Value::String(s) => Some(s.len()),
                    _ => None,
                };
                collection_size.map_or(false, |s| s <= *max_size)
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
                    serde_json::Value::Array(arr) => Some(arr.len()),
                    serde_json::Value::Object(obj) => Some(obj.len()),
                    serde_json::Value::String(s) => Some(s.len()),
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
        value: serde_json::Value
    }
    impl {
        fn check(arr: &Value, value: &serde_json::Value) -> bool {
            {
                if let serde_json::Value::Array(array) = arr {
                    array.contains(value)
                } else {
                    false
                }
            }
        }
        fn error(value: &serde_json::Value) -> String {
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
                match value {
                    serde_json::Value::Array(arr) => arr.is_empty(),
                    serde_json::Value::Object(obj) => obj.is_empty(),
                    serde_json::Value::String(s) => s.is_empty(),
                    _ => false,
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
                match value {
                    serde_json::Value::Array(arr) => !arr.is_empty(),
                    serde_json::Value::Object(obj) => !obj.is_empty(),
                    serde_json::Value::String(s) => !s.is_empty(),
                    _ => false,
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
                if let serde_json::Value::Array(arr) = value {
                    let mut seen = std::collections::HashSet::new();
                    for item in arr {
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
validator_fn!(pub fn array_contains(value: serde_json::Value) -> ArrayContains);
validator_fn!(pub fn empty() -> Empty);
validator_fn!(pub fn not_empty() -> NotEmpty);
validator_fn!(pub fn unique() -> Unique);

// Specialized convenience functions
pub fn contains_value(value: serde_json::Value) -> ArrayContains {
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