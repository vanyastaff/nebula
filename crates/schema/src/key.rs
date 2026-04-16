use serde::{Deserialize, Serialize};

use crate::SchemaError;

/// Stable identifier for a schema field.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldKey(String);

impl FieldKey {
    /// Create and validate a field key.
    ///
    /// Rules:
    /// - non-empty
    /// - max 64 chars
    /// - starts with ASCII letter or underscore
    /// - contains only ASCII alphanumeric chars or underscore
    pub fn new(value: impl Into<String>) -> Result<Self, SchemaError> {
        let value = value.into();
        let bytes = value.as_bytes();

        if value.is_empty() {
            return Err(SchemaError::InvalidKey("key cannot be empty".to_owned()));
        }
        if value.len() > 64 {
            return Err(SchemaError::InvalidKey("key max 64 chars".to_owned()));
        }

        let first = bytes[0] as char;
        if !first.is_ascii_alphabetic() && first != '_' {
            return Err(SchemaError::InvalidKey(
                "key must start with letter or underscore".to_owned(),
            ));
        }

        if !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(SchemaError::InvalidKey(
                "key must be ASCII alphanumeric or underscore".to_owned(),
            ));
        }

        Ok(Self(value))
    }

    /// Borrow key as string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&'static str> for FieldKey {
    fn from(value: &'static str) -> Self {
        Self::new(value).expect("invalid static FieldKey")
    }
}

#[cfg(test)]
mod tests {
    use super::FieldKey;

    #[test]
    fn accepts_valid_keys() {
        assert!(FieldKey::new("alpha").is_ok());
        assert!(FieldKey::new("_leading_underscore").is_ok());
        assert!(FieldKey::new("a1_b2").is_ok());
    }

    #[test]
    fn rejects_invalid_keys() {
        assert!(FieldKey::new("").is_err());
        assert!(FieldKey::new("1starts_with_digit").is_err());
        assert!(FieldKey::new("has-dash").is_err());
        assert!(FieldKey::new("contains space").is_err());
    }
}
