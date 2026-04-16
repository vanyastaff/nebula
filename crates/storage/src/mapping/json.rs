//! JSON serialization helpers for storage.
//!
//! Thin wrappers around `serde_json` that return [`StorageError`] instead
//! of `serde_json::Error`, keeping error handling uniform across the crate.

use crate::error::StorageError;

/// Serialize a value to a `serde_json::Value`.
///
/// Suitable for `JSONB` columns (Postgres) or `TEXT`/`JSON` columns (SQLite).
///
/// # Errors
///
/// Returns [`StorageError::Serialization`] if the value cannot be serialized.
pub fn to_json<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, StorageError> {
    serde_json::to_value(value).map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Deserialize a typed value from a `serde_json::Value`.
///
/// # Errors
///
/// Returns [`StorageError::Serialization`] if deserialization fails.
pub fn from_json<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> Result<T, StorageError> {
    serde_json::from_value(value).map_err(|e| StorageError::Serialization(e.to_string()))
}

/// Deserialize from an optional JSON value, returning `None` for `None` or
/// JSON `null`.
///
/// # Errors
///
/// Returns [`StorageError::Serialization`] if the inner value exists but
/// cannot be deserialized.
pub fn from_json_opt<T: serde::de::DeserializeOwned>(
    value: Option<serde_json::Value>,
) -> Result<Option<T>, StorageError> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => from_json(v).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Sample {
        name: String,
        count: i32,
    }

    #[test]
    fn roundtrip_struct() {
        let original = Sample {
            name: "test".into(),
            count: 42,
        };
        let json = to_json(&original).unwrap();
        let restored: Sample = from_json(json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn roundtrip_map() {
        let mut map = std::collections::HashMap::new();
        map.insert("key".to_string(), "value".to_string());
        let json = to_json(&map).unwrap();
        let restored: std::collections::HashMap<String, String> = from_json(json).unwrap();
        assert_eq!(map, restored);
    }

    #[test]
    fn from_json_opt_none() {
        let result: Option<Sample> = from_json_opt(None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn from_json_opt_null() {
        let result: Option<Sample> = from_json_opt(Some(serde_json::Value::Null)).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn from_json_opt_some() {
        let original = Sample {
            name: "opt".into(),
            count: 7,
        };
        let json = to_json(&original).unwrap();
        let result: Option<Sample> = from_json_opt(Some(json)).unwrap();
        assert_eq!(result.unwrap(), original);
    }

    #[test]
    fn from_json_type_mismatch() {
        let json = serde_json::json!("a plain string");
        let err = from_json::<Sample>(json).unwrap_err();
        assert!(err.to_string().contains("serialization"));
    }
}
