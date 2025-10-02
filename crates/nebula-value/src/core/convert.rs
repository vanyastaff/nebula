//! Conversion utilities and extension traits for Value
//!
//! This module provides idiomatic conversion between nebula_value::Value
//! and serde_json::Value with minimal allocations.

#![cfg(feature = "serde")]

use crate::core::value::Value;

/// Extension trait for `&Value` providing conversion to `serde_json::Value`
/// without unnecessary cloning when the reference is already available.
pub trait ValueRefExt {
    /// Convert a reference to Value into serde_json::Value
    ///
    /// This is optimized to avoid cloning when possible:
    /// - For Array/Object: reuses internal serde_json::Value storage
    /// - For scalars: creates serde_json::Value directly without intermediate clone
    fn to_json(&self) -> serde_json::Value;
}

impl ValueRefExt for Value {
    fn to_json(&self) -> serde_json::Value {
        // Optimized: work directly with references, avoid cloning Value
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Boolean(b) => serde_json::Value::Bool(*b),
            Value::Integer(i) => serde_json::Value::Number(i.value().into()),
            Value::Float(f) => {
                serde_json::Number::from_f64(f.value())
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            }
            Value::Decimal(d) => serde_json::Value::String(d.to_string()),
            Value::Text(t) => serde_json::Value::String(t.as_str().to_string()),
            Value::Bytes(b) => {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(b.as_slice());
                serde_json::Value::String(encoded)
            }
            // Array and Object already store serde_json::Value, so we collect their iterators
            Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().cloned().collect())
            }
            Value::Object(obj) => {
                serde_json::Value::Object(
                    obj.entries().map(|(k, v)| (k.clone(), v.clone())).collect()
                )
            }
            #[cfg(feature = "temporal")]
            Value::Date(d) => serde_json::Value::String(d.to_iso_string().to_string()),
            #[cfg(feature = "temporal")]
            Value::Time(t) => serde_json::Value::String(t.to_iso_string().to_string()),
            #[cfg(feature = "temporal")]
            Value::DateTime(dt) => serde_json::Value::String(dt.to_iso_string().to_string()),
            #[cfg(feature = "temporal")]
            Value::Duration(dur) => serde_json::Value::Number((dur.as_millis() as u64).into()),
        }
    }
}

/// Extension trait for `&serde_json::Value` providing conversion to `nebula_value::Value`
pub trait JsonValueExt {
    /// Convert a reference to serde_json::Value into nebula_value::Value
    ///
    /// Returns None if the conversion fails (which should be rare for valid JSON).
    fn to_nebula_value(&self) -> Option<Value>;

    /// Convert a reference to serde_json::Value into nebula_value::Value
    ///
    /// Returns Value::Null if the conversion fails.
    fn to_nebula_value_or_null(&self) -> Value;
}

impl JsonValueExt for serde_json::Value {
    #[inline]
    fn to_nebula_value(&self) -> Option<Value> {
        Value::try_from(self.clone()).ok()
    }

    #[inline]
    fn to_nebula_value_or_null(&self) -> Value {
        Value::try_from(self.clone()).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_value_ref_to_json() {
        let value = Value::integer(42);
        let json = value.to_json();
        assert_eq!(json, json!(42));
    }

    #[test]
    fn test_json_value_to_nebula() {
        let json = json!({"key": "value"});
        let value = json.to_nebula_value().unwrap();
        assert!(value.is_object());
    }

    #[test]
    fn test_json_value_to_nebula_or_null() {
        let json = json!("text");
        let value = json.to_nebula_value_or_null();
        assert!(value.is_text());
    }

    #[test]
    fn test_roundtrip() {
        let original = Value::text("hello");
        let json = original.to_json();
        let converted = json.to_nebula_value_or_null();
        assert_eq!(original.as_str(), converted.as_str());
    }
}
