//! Serde serialization and deserialization for Value
//!
//! This module provides efficient JSON serialization/deserialization
//! that preserves all Value types.

use crate::collections::{Array, Object};
use crate::core::value::Value;
use crate::scalar::{Float, Integer, Text};
use base64::Engine;
use rust_decimal::Decimal;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

impl Serialize for Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Value::Null => serializer.serialize_none(),

            Value::Boolean(b) => serializer.serialize_bool(*b),

            Value::Integer(i) => serializer.serialize_i64(i.value()),

            Value::Float(f) => {
                if f.is_nan() {
                    // Serialize NaN as null (JSON doesn't support NaN)
                    serializer.serialize_none()
                } else if f.is_infinite() {
                    // Serialize Infinity as string
                    if f.is_positive_infinity() {
                        serializer.serialize_str("+Infinity")
                    } else {
                        serializer.serialize_str("-Infinity")
                    }
                } else {
                    serializer.serialize_f64(f.value())
                }
            }

            Value::Decimal(d) => {
                // Serialize as string to preserve precision
                serializer.serialize_str(&d.to_string())
            }

            Value::Text(t) => serializer.serialize_str(t.as_str()),

            Value::Bytes(b) => {
                // Serialize as base64 string
                let encoded = base64::engine::general_purpose::STANDARD.encode(b.as_slice());
                serializer.serialize_str(&encoded)
            }

            Value::Array(arr) => {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(Some(arr.len()))?;
                for item in arr.iter() {
                    // Array stores serde_json::Value internally for efficiency
                    seq.serialize_element(item)?;
                }
                seq.end()
            }

            Value::Object(obj) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(obj.len()))?;
                for (key, value) in obj.entries() {
                    // Object stores serde_json::Value internally for efficiency
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }

            #[cfg(feature = "temporal")]
            Value::Date(d) => serializer.serialize_str(d.to_iso_string()),

            #[cfg(feature = "temporal")]
            Value::Time(t) => serializer.serialize_str(t.to_iso_string()),

            #[cfg(feature = "temporal")]
            Value::DateTime(dt) => serializer.serialize_str(dt.to_iso_string()),

            #[cfg(feature = "temporal")]
            Value::Duration(dur) => {
                // Serialize as milliseconds
                serializer.serialize_u64(dur.as_millis() as u64)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Boolean(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Integer(Integer::new(v)))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if v <= i64::MAX as u64 {
            Ok(Value::Integer(Integer::new(v as i64)))
        } else {
            // Convert large u64 to float
            Ok(Value::Float(Float::new(v as f64)))
        }
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Float(Float::new(v)))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // Try to parse as special values
        match v {
            "+Infinity" => Ok(Value::Float(Float::new(f64::INFINITY))),
            "-Infinity" => Ok(Value::Float(Float::new(f64::NEG_INFINITY))),
            "NaN" => Ok(Value::Float(Float::new(f64::NAN))),
            _ => {
                // Try to parse as decimal first
                if let Ok(decimal) = v.parse::<Decimal>() {
                    // Only treat as decimal if it has decimal point or is very large
                    if v.contains('.') || v.contains('e') || v.contains('E') {
                        return Ok(Value::Decimal(decimal));
                    }
                }

                // Otherwise treat as text
                Ok(Value::Text(Text::new(v.to_string())))
            }
        }
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&v)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut arr = Array::new();

        while let Some(elem) = seq.next_element::<serde_json::Value>()? {
            arr = arr.push(elem);
        }

        Ok(Value::Array(arr))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut obj = Object::new();

        while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
            obj = obj.insert(key, value);
        }

        Ok(Value::Object(obj))
    }
}

/// Convert Value to serde_json::Value for compatibility
impl From<Value> for serde_json::Value {
    fn from(value: Value) -> Self {
        match value {
            Value::Null => serde_json::Value::Null,
            Value::Boolean(b) => serde_json::Value::Bool(b),
            Value::Integer(i) => serde_json::Value::Number(i.value().into()),
            Value::Float(f) => {
                if let Some(n) = serde_json::Number::from_f64(f.value()) {
                    serde_json::Value::Number(n)
                } else {
                    // NaN or Infinity - use null
                    serde_json::Value::Null
                }
            }
            Value::Decimal(d) => serde_json::Value::String(d.to_string()),
            Value::Text(t) => serde_json::Value::String(t.to_string()),
            Value::Bytes(b) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(b.as_slice());
                serde_json::Value::String(encoded)
            }
            Value::Array(arr) => {
                // Recursively convert each array element
                let vec: Vec<serde_json::Value> = arr
                    .iter()
                    .map(|item| serde_json::Value::from(item.clone()))
                    .collect();
                serde_json::Value::Array(vec)
            }
            Value::Object(obj) => {
                // Recursively convert each object value
                let map: serde_json::Map<String, serde_json::Value> = obj
                    .entries()
                    .map(|(k, v)| (k.clone(), serde_json::Value::from(v.clone())))
                    .collect();
                serde_json::Value::Object(map)
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

// NOTE: TryFrom<serde_json::Value> and From<serde_json::Value> are now defined
// in core/conversions.rs (always available, not gated by "serde" feature)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_null() {
        let val = Value::Null;
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "null");
    }

    #[test]
    fn test_serialize_boolean() {
        let val = Value::Boolean(true);
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "true");
    }

    #[test]
    fn test_serialize_integer() {
        let val = Value::integer(42);
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "42");
    }

    #[test]
    fn test_serialize_float() {
        let val = Value::float(3.14);
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "3.14");
    }

    #[test]
    fn test_serialize_nan() {
        let val = Value::float(f64::NAN);
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "null");
    }

    #[test]
    fn test_serialize_infinity() {
        let val = Value::float(f64::INFINITY);
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "\"+Infinity\"");
    }

    #[test]
    fn test_serialize_text() {
        let val = Value::text("hello");
        let json = serde_json::to_string(&val).unwrap();
        assert_eq!(json, "\"hello\"");
    }

    #[test]
    fn test_serialize_bytes() {
        let val = Value::bytes(vec![1, 2, 3]);
        let json = serde_json::to_string(&val).unwrap();
        // Base64 of [1, 2, 3] is "AQID"
        assert_eq!(json, "\"AQID\"");
    }

    #[test]
    fn test_deserialize_null() {
        let json = "null";
        let val: Value = serde_json::from_str(json).unwrap();
        assert!(val.is_null());
    }

    #[test]
    fn test_deserialize_boolean() {
        let json = "true";
        let val: Value = serde_json::from_str(json).unwrap();
        assert_eq!(val.as_boolean(), Some(true));
    }

    #[test]
    fn test_deserialize_integer() {
        let json = "42";
        let val: Value = serde_json::from_str(json).unwrap();
        assert_eq!(val.as_integer(), Some(42));
    }

    #[test]
    fn test_deserialize_float() {
        let json = "3.14";
        let val: Value = serde_json::from_str(json).unwrap();
        assert_eq!(val.as_float(), Some(3.14));
    }

    #[test]
    fn test_deserialize_string() {
        let json = "\"hello\"";
        let val: Value = serde_json::from_str(json).unwrap();
        assert_eq!(val.as_str(), Some("hello"));
    }

    #[test]
    fn test_roundtrip_simple() {
        let original = Value::integer(42);
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(original.as_integer(), deserialized.as_integer());
    }

    #[test]
    fn test_roundtrip_text() {
        let original = Value::text("hello world");
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(original.as_str(), deserialized.as_str());
    }

    #[test]
    fn test_from_json_value() {
        let json = serde_json::json!({
            "name": "Alice",
            "age": 30,
            "active": true
        });

        let val = Value::try_from(json).unwrap();
        assert!(val.is_object());
    }

    #[test]
    fn test_to_json_value() {
        let val = Value::integer(42);
        let json: serde_json::Value = val.into();
        assert_eq!(json, serde_json::json!(42));
    }
}
