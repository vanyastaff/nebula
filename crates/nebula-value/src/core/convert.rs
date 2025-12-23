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
            Value::Float(f) => serde_json::Number::from_f64(f.value())
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Decimal(d) => serde_json::Value::String(d.to_string()),
            Value::Text(t) => serde_json::Value::String(t.as_str().to_string()),
            Value::Bytes(b) => {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(b.as_slice());
                serde_json::Value::String(encoded)
            }
            // Array and Object need to convert nested Values to serde_json::Value
            Value::Array(arr) => {
                // Pre-allocate capacity to avoid multiple reallocations
                let mut vec = Vec::with_capacity(arr.len());
                vec.extend(arr.iter().map(|v| v.to_json()));
                serde_json::Value::Array(vec)
            }
            Value::Object(obj) => {
                // Pre-allocate capacity to avoid rehashing
                let entries = obj.entries();
                let (size_hint, _) = entries.size_hint();
                let mut map = serde_json::Map::with_capacity(size_hint);
                map.extend(entries.map(|(k, v)| (k.clone(), v.to_json())));
                serde_json::Value::Object(map)
            },
            #[cfg(feature = "temporal")]
            Value::Date(d) => serde_json::Value::String(d.to_iso_string().to_string()),
            #[cfg(feature = "temporal")]
            Value::Time(t) => serde_json::Value::String(t.to_iso_string().to_string()),
            #[cfg(feature = "temporal")]
            Value::DateTime(dt) => serde_json::Value::String(dt.to_iso_string().to_string()),
            #[cfg(feature = "temporal")]
            Value::Duration(dur) => {
                // Clamp to u64::MAX for unrealistically large durations (>584 million years)
                let millis = dur.as_millis().min(u64::MAX as u128) as u64;
                serde_json::Value::Number(millis.into())
            }
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
        Some(Value::from(self.clone()))
    }

    #[inline]
    fn to_nebula_value_or_null(&self) -> Value {
        Value::from(self.clone())
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
