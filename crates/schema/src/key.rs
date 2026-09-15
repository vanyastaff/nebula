//! Stable identifier for a schema field. No panicking constructors.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{error::ValidationError, path::FieldPath};

/// Stable field identifier. Cheap to clone (Arc-backed).
///
/// # Examples
///
/// ```
/// use nebula_schema::field_key;
/// let k = field_key!("alpha");
/// assert_eq!(k.as_str(), "alpha");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct FieldKey(Arc<str>);

/// Checked static literal used by generated code before allocating a key.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct LiteralFieldKey(&'static str);

impl LiteralFieldKey {
    /// Check a static literal without allocating or panicking.
    #[must_use]
    pub const fn parse(value: &'static str) -> Option<Self> {
        if key_error(value).is_some() {
            None
        } else {
            Some(Self(value))
        }
    }
}

const fn key_error(value: &str) -> Option<&'static str> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return Some("key cannot be empty");
    }
    if bytes.len() > 64 {
        return Some("key max 64 chars");
    }
    if !bytes[0].is_ascii_alphabetic() && bytes[0] != b'_' {
        return Some("key must start with letter or underscore");
    }
    let mut index = 1;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphanumeric() && bytes[index] != b'_' {
            return Some("key must be ASCII alphanumeric or underscore");
        }
        index += 1;
    }
    None
}

impl FieldKey {
    /// Build a field key from a candidate string.
    ///
    /// Rules:
    /// - non-empty
    /// - max 64 chars
    /// - starts with ASCII letter or underscore
    /// - only ASCII alphanumeric or underscore afterwards
    ///
    /// # Errors
    ///
    /// Returns `invalid_key` when the candidate string violates key format constraints.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ValidationError> {
        let value = value.as_ref();
        if let Some(message) = key_error(value) {
            return Err(Self::err(value, message));
        }

        Ok(Self(Arc::from(value)))
    }

    pub(crate) fn from_validated_literal(value: LiteralFieldKey) -> Self {
        Self(Arc::from(value.0))
    }

    /// Borrow the key as `&str`.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Access the underlying `Arc<str>` handle.
    #[inline]
    #[must_use]
    pub const fn as_arc(&self) -> &Arc<str> {
        &self.0
    }

    fn err(value: &str, msg: &'static str) -> ValidationError {
        Self::err_at(FieldPath::root(), value, msg)
    }

    pub(crate) fn err_at(path: FieldPath, value: &str, msg: &'static str) -> ValidationError {
        ValidationError::invalid_key(path, value, msg)
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
            assert_eq!(err.code(), "invalid_key");
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
