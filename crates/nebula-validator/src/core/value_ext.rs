//! Extension traits for nebula_value::Value to provide serde_json-compatible API

use nebula_value::Value;

/// Extension trait to provide serde_json-compatible methods for nebula_value::Value
pub trait ValueExt {
    /// Get value as f64 (compatible with serde_json API)
    fn as_f64(&self) -> Option<f64>;

    /// Get value as i64 (compatible with serde_json API)
    fn as_i64(&self) -> Option<i64>;

    /// Get value as u64 (compatible with serde_json API)
    fn as_u64(&self) -> Option<u64>;

    /// Check if value is number (compatible with serde_json API)
    fn is_number(&self) -> bool;

    /// Check if value is boolean (compatible with serde_json API)
    fn is_boolean(&self) -> bool;

    /// Get numeric value as f64 (handles both int and float)
    fn as_numeric(&self) -> Option<f64>;

    /// Get length of collection or string
    fn len(&self) -> usize;

    /// Check if collection or string is empty
    fn is_empty(&self) -> bool;

    /// Check if object contains a key
    fn contains_key(&self, key: &str) -> bool;

    /// Check if array or object contains a value
    fn contains(&self, value: &serde_json::Value) -> bool;
}

impl ValueExt for Value {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(f.value()),
            Value::Integer(i) => Some(i.value() as f64),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(i.value()),
            Value::Float(f) => {
                let val = f.value();
                if val.fract() == 0.0 && val >= i64::MIN as f64 && val <= i64::MAX as f64 {
                    Some(val as i64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Integer(i) => {
                let val = i.value();
                if val >= 0 {
                    Some(val as u64)
                } else {
                    None
                }
            }
            Value::Float(f) => {
                let val = f.value();
                if val >= 0.0 && val.fract() == 0.0 && val <= u64::MAX as f64 {
                    Some(val as u64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn is_number(&self) -> bool {
        matches!(self, Value::Integer(_) | Value::Float(_))
    }

    fn is_boolean(&self) -> bool {
        matches!(self, Value::Boolean(_))
    }

    fn as_numeric(&self) -> Option<f64> {
        self.as_f64()
    }

    fn len(&self) -> usize {
        match self {
            Value::Text(t) => t.len(),
            Value::Array(a) => a.len(),
            Value::Object(o) => o.len(),
            Value::Bytes(b) => b.as_slice().len(),
            _ => 0,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Value::Text(t) => t.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            Value::Bytes(b) => b.is_empty(),
            _ => false,
        }
    }

    fn contains_key(&self, key: &str) -> bool {
        match self {
            Value::Object(o) => o.contains_key(key),
            _ => false,
        }
    }

    fn contains(&self, value: &serde_json::Value) -> bool {
        match self {
            Value::Array(a) => a.iter().any(|v| v == value),
            _ => false,
        }
    }
}
