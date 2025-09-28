//! Extension traits for nebula_value::Value to provide serde_json-compatible API

use nebula_value::Value;

/// Extension trait to provide serde_json-compatible methods for nebula_value::Value
pub trait ValueExt {
    /// Get value as f64 (compatible with serde_json API)
    fn as_f64(&self) -> Option<f64>;

    /// Get value as u64 (compatible with serde_json API)
    fn as_u64(&self) -> Option<u64>;

    /// Check if value is number (compatible with serde_json API)
    fn is_number(&self) -> bool;

    /// Check if value is boolean (compatible with serde_json API)
    fn is_boolean(&self) -> bool;

    /// Get numeric value as f64 (handles both int and float)
    fn as_numeric(&self) -> Option<f64>;
}

impl ValueExt for Value {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(f.get()),
            Value::Int(i) => Some(i.get() as f64),
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Int(i) => {
                let val = i.get();
                if val >= 0 {
                    Some(val as u64)
                } else {
                    None
                }
            }
            Value::Float(f) => {
                let val = f.get();
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
        matches!(self, Value::Int(_) | Value::Float(_))
    }

    fn is_boolean(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    fn as_numeric(&self) -> Option<f64> {
        self.as_f64()
    }
}