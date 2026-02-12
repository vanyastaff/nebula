//! Collection structure validators (for maps)

use crate::core::{Validate, ValidationError, ValidatorMetadata};
use std::collections::HashMap;
use std::hash::Hash;
use std::marker::PhantomData;

// ============================================================================
// HAS KEY
// ============================================================================

/// Validates that a map has a specific key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HasKey<K, V> {
    /// The key that must exist in the map.
    pub key: K,
    _phantom: PhantomData<V>,
}

impl<K, V> HasKey<K, V> {
    pub fn new(key: K) -> Self {
        Self {
            key,
            _phantom: PhantomData,
        }
    }
}

impl<K, V> Validate for HasKey<K, V>
where
    K: Hash + Eq,
{
    type Input = HashMap<K, V>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.contains_key(&self.key) {
            Ok(())
        } else {
            Err(ValidationError::new("has_key", "Map must contain the key"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("HasKey")
            .with_tag("collection")
            .with_tag("map")
    }
}

pub fn has_key<K, V>(key: K) -> HasKey<K, V>
where
    K: Hash + Eq,
{
    HasKey::new(key)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_key() {
        let mut map = HashMap::new();
        map.insert("name", "John");
        map.insert("email", "john@example.com");

        let validator = has_key("email");
        assert!(validator.validate(&map).is_ok());

        let validator = has_key("phone");
        assert!(validator.validate(&map).is_err());
    }
}
