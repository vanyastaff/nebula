//! Utility functions for working with serde_json::Value

use serde_json::{Number, Value};

/// Get the type name of a Value for error messages
pub fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Extract i64 from Number, trying both i64 and f64 representations
#[inline]
pub fn number_as_i64(num: &Number) -> Option<i64> {
    num.as_i64().or_else(|| num.as_f64().map(|f| f as i64))
}

/// Extract f64 from Number, trying both f64 and i64 representations
#[inline]
pub fn number_as_f64(num: &Number) -> Option<f64> {
    num.as_f64().or_else(|| num.as_i64().map(|i| i as f64))
}

/// Check if two numbers can be added as integers
#[inline]
pub fn can_add_as_int(l: &Number, r: &Number) -> bool {
    l.is_i64() && r.is_i64()
}

/// Check if a number represents an integer
#[inline]
pub fn is_integer_number(num: &Number) -> bool {
    num.is_i64() || num.is_u64()
}

/// Check if a value is numeric (number type)
#[inline]
pub fn is_numeric(value: &Value) -> bool {
    value.is_number()
}

/// Check if a value is truthy (not null, false, 0, or empty string)
pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i != 0
            } else if let Some(f) = n.as_f64() {
                f != 0.0 && !f.is_nan()
            } else {
                true // u64 values
            }
        }
        Value::String(s) => !s.is_empty(),
        Value::Array(arr) => !arr.is_empty(),
        Value::Object(obj) => !obj.is_empty(),
    }
}

/// Convert Value to boolean (truthy/falsy semantics)
pub fn to_boolean(value: &Value) -> bool {
    is_truthy(value)
}

/// Convert Value to i64 with error
pub fn to_integer(value: &Value) -> Result<i64, &'static str> {
    match value {
        Value::Number(n) => number_as_i64(n).ok_or("number is not an integer"),
        Value::String(s) => s.parse().map_err(|_| "string is not a valid integer"),
        Value::Bool(b) => Ok(if *b { 1 } else { 0 }),
        _ => Err("value cannot be converted to integer"),
    }
}

/// Convert Value to f64 with error
pub fn to_float(value: &Value) -> Result<f64, &'static str> {
    match value {
        Value::Number(n) => number_as_f64(n).ok_or("number cannot be represented as float"),
        Value::String(s) => s.parse().map_err(|_| "string is not a valid number"),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        _ => Err("value cannot be converted to number"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_type_name() {
        assert_eq!(value_type_name(&Value::Null), "null");
        assert_eq!(value_type_name(&Value::Bool(true)), "boolean");
        assert_eq!(value_type_name(&Value::Number(42.into())), "number");
        assert_eq!(
            value_type_name(&Value::String("test".to_string())),
            "string"
        );
        assert_eq!(value_type_name(&Value::Array(vec![])), "array");
        assert_eq!(
            value_type_name(&Value::Object(serde_json::Map::new())),
            "object"
        );
    }

    #[test]
    fn test_is_truthy() {
        assert!(!is_truthy(&Value::Null));
        assert!(!is_truthy(&Value::Bool(false)));
        assert!(is_truthy(&Value::Bool(true)));
        assert!(!is_truthy(&Value::Number(0.into())));
        assert!(is_truthy(&Value::Number(1.into())));
        assert!(!is_truthy(&Value::String(String::new())));
        assert!(is_truthy(&Value::String("test".to_string())));
    }
}
