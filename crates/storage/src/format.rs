//! Serialization format abstraction for storage backends.
//!
//! Allows switching between JSON (human-readable) and MessagePack (compact)
//! for persisted data without changing storage logic.

use serde::{Serialize, de::DeserializeOwned};

/// Storage serialization format.
///
/// JSON is the default (human-readable, debuggable). MessagePack is available
/// behind the `msgpack-storage` feature for ~30-50% smaller payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StorageFormat {
    /// JSON via serde_json (default).
    #[default]
    Json,
    /// MessagePack via rmp-serde (compact binary).
    #[cfg(feature = "msgpack-storage")]
    MessagePack,
}

impl StorageFormat {
    /// Serialize a value to bytes in this format.
    ///
    /// # Errors
    ///
    /// Returns a string error if serialization fails.
    pub fn serialize<T: Serialize>(&self, value: &T) -> Result<Vec<u8>, String> {
        match self {
            Self::Json => serde_json::to_vec(value).map_err(|e| e.to_string()),
            #[cfg(feature = "msgpack-storage")]
            Self::MessagePack => rmp_serde::to_vec(value).map_err(|e| e.to_string()),
        }
    }

    /// Deserialize a value from bytes in this format.
    ///
    /// # Errors
    ///
    /// Returns a string error if deserialization fails.
    pub fn deserialize<T: DeserializeOwned>(&self, data: &[u8]) -> Result<T, String> {
        match self {
            Self::Json => serde_json::from_slice(data).map_err(|e| e.to_string()),
            #[cfg(feature = "msgpack-storage")]
            Self::MessagePack => rmp_serde::from_slice(data).map_err(|e| e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip() {
        let data = serde_json::json!({"key": "value", "num": 42});
        let bytes = StorageFormat::Json.serialize(&data).unwrap();
        let parsed: serde_json::Value = StorageFormat::Json.deserialize(&bytes).unwrap();
        assert_eq!(data, parsed);
    }

    #[cfg(feature = "msgpack-storage")]
    #[test]
    fn msgpack_roundtrip() {
        let data = serde_json::json!({"key": "value", "num": 42});
        let bytes = StorageFormat::MessagePack.serialize(&data).unwrap();
        let parsed: serde_json::Value = StorageFormat::MessagePack.deserialize(&bytes).unwrap();
        assert_eq!(data, parsed);
        // MessagePack should be smaller than JSON
        let json_bytes = StorageFormat::Json.serialize(&data).unwrap();
        assert!(bytes.len() < json_bytes.len());
    }

    #[test]
    fn default_is_json() {
        assert_eq!(StorageFormat::default(), StorageFormat::Json);
    }
}
