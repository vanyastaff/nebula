//! Conversion utilities between serde_json::Value and nebula_value::Value

use nebula_value::Value as NebulaValue;
use serde_json::Value as JsonValue;

/// Convert serde_json::Value to nebula_value::Value
pub fn json_to_nebula(value: &JsonValue) -> NebulaValue {
    match value {
        JsonValue::Null => NebulaValue::Null,
        JsonValue::Bool(b) => NebulaValue::boolean(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                NebulaValue::integer(i)
            } else if let Some(f) = n.as_f64() {
                NebulaValue::float(f)
            } else {
                NebulaValue::Null
            }
        }
        JsonValue::String(s) => NebulaValue::text(s.clone()),
        JsonValue::Array(arr) => {
            let nebula_arr: Vec<serde_json::Value> = arr.clone();
            NebulaValue::Array(nebula_value::Array::from(nebula_arr))
        }
        JsonValue::Object(obj) => {
            let map: std::collections::HashMap<String, serde_json::Value> =
                obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            NebulaValue::Object(map.into_iter().collect())
        }
    }
}

/// Convert nebula_value::Value to serde_json::Value
pub fn nebula_to_json(value: &NebulaValue) -> JsonValue {
    match value {
        NebulaValue::Null => JsonValue::Null,
        NebulaValue::Boolean(b) => JsonValue::Bool(*b),
        NebulaValue::Integer(i) => JsonValue::Number(i.value().into()),
        NebulaValue::Float(f) => {
            serde_json::Number::from_f64(f.value())
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null)
        }
        NebulaValue::Text(s) => JsonValue::String(s.to_string()),
        NebulaValue::Bytes(b) => {
            // Encode bytes as hex string
            JsonValue::String(base64_encode(b.as_slice()))
        }
        NebulaValue::DateTime(dt) => JsonValue::String(dt.to_string()),
        NebulaValue::Array(arr) => {
            // Array internally uses serde_json::Value
            let items: Vec<serde_json::Value> = (0..arr.len())
                .map(|i| arr.get(i).cloned().unwrap_or(serde_json::Value::Null))
                .collect();
            JsonValue::Array(items)
        }
        NebulaValue::Object(obj) => {
            // Object internally uses serde_json::Value
            let map: serde_json::Map<String, serde_json::Value> =
                obj.keys()
                    .map(|k| (k.clone(), obj.get(k).cloned().unwrap_or(serde_json::Value::Null)))
                    .collect();
            JsonValue::Object(map)
        }
    }
}

/// Simple base64 encoding helper
fn base64_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::new();
    for byte in bytes {
        write!(&mut result, "{:02x}", byte).unwrap();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_to_nebula_primitives() {
        assert!(matches!(json_to_nebula(&json!(null)), NebulaValue::Null));
        assert!(matches!(
            json_to_nebula(&json!(true)),
            NebulaValue::Boolean(_)
        ));
        assert!(matches!(
            json_to_nebula(&json!(42)),
            NebulaValue::Integer(_)
        ));
        assert!(matches!(
            json_to_nebula(&json!(3.14)),
            NebulaValue::Float(_)
        ));
        assert!(matches!(
            json_to_nebula(&json!("hello")),
            NebulaValue::Text(_)
        ));
    }

    #[test]
    fn test_json_to_nebula_array() {
        let result = json_to_nebula(&json!([1, 2, 3]));
        assert!(matches!(result, NebulaValue::Array(_)));
    }

    #[test]
    fn test_json_to_nebula_object() {
        let result = json_to_nebula(&json!({"key": "value"}));
        assert!(matches!(result, NebulaValue::Object(_)));
    }

    #[test]
    fn test_nebula_to_json_primitives() {
        assert_eq!(nebula_to_json(&NebulaValue::Null), json!(null));
        assert_eq!(
            nebula_to_json(&NebulaValue::boolean(true)),
            json!(true)
        );
        assert_eq!(nebula_to_json(&NebulaValue::integer(42)), json!(42));
        assert_eq!(nebula_to_json(&NebulaValue::text("hello")), json!("hello"));
    }

    #[test]
    fn test_roundtrip() {
        let original = json!({"name": "test", "value": 42, "enabled": true});
        let nebula = json_to_nebula(&original);
        let back = nebula_to_json(&nebula);
        assert_eq!(original, back);
    }
}
