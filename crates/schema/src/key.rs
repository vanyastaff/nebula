//! Stable identifier for a schema field. No panicking constructors.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{error::ValidationError, path::FieldPath};

/// Stable field identifier. Cheap to clone (Arc-backed).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct FieldKey(Arc<str>);

impl FieldKey {
    /// Build a field key from a candidate string.
    ///
    /// Rules:
    /// - non-empty
    /// - max 64 chars
    /// - starts with ASCII letter or underscore
    /// - only ASCII alphanumeric or underscore afterwards
    pub fn new(value: impl AsRef<str>) -> Result<Self, ValidationError> {
        let value = value.as_ref();
        let bytes = value.as_bytes();

        if value.is_empty() {
            return Err(Self::err(value, "key cannot be empty"));
        }
        if value.len() > 64 {
            return Err(Self::err(value, "key max 64 chars"));
        }
        let first = bytes[0] as char;
        if !first.is_ascii_alphabetic() && first != '_' {
            return Err(Self::err(value, "key must start with letter or underscore"));
        }
        if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(Self::err(
                value,
                "key must be ASCII alphanumeric or underscore",
            ));
        }

        Ok(Self(Arc::from(value)))
    }

    /// Borrow the key as `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Access the underlying `Arc<str>` handle.
    pub fn as_arc(&self) -> &Arc<str> {
        &self.0
    }

    fn err(value: &str, msg: &'static str) -> ValidationError {
        ValidationError::new("invalid_key")
            .at(FieldPath::root())
            .message(msg)
            .param("key", value.to_owned())
            .build()
    }
}

impl std::fmt::Display for FieldKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for FieldKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for FieldKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for FieldKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_keys() {
        assert!(FieldKey::new("alpha").is_ok());
        assert!(FieldKey::new("_leading_underscore").is_ok());
        assert!(FieldKey::new("a1_b2").is_ok());
    }

    #[test]
    fn rejects_invalid_keys() {
        for bad in ["", "1bad", "has-dash", "has space", &"x".repeat(65)] {
            let err = FieldKey::new(bad).unwrap_err();
            assert_eq!(err.code, "invalid_key");
        }
    }

    #[test]
    fn deserialize_rejects_invalid() {
        let invalid = "\"has-dash\"";
        let r: Result<FieldKey, _> = serde_json::from_str(invalid);
        assert!(r.is_err());
    }

    #[test]
    fn clone_is_cheap() {
        let k = FieldKey::new("field").unwrap();
        let c1 = k.clone();
        let c2 = k.clone();
        assert_eq!(k.as_str(), c1.as_str());
        assert_eq!(c1.as_str(), c2.as_str());
    }
}
