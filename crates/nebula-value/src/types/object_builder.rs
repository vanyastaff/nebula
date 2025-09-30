//! ObjectBuilder - efficient construction of objects with batch operations
//!
//! This module provides a builder pattern for Object construction.

use std::collections::BTreeMap;
use crate::types::{Object, ObjectError};
use crate::{Value, ValueLimits};

/// Builder for efficient object construction
///
/// # Example
///
/// ```
/// use nebula_value::{ObjectBuilder, Value};
///
/// let object = ObjectBuilder::new()
///     .insert("name", Value::string("Alice"))
///     .insert("age", Value::int(30))
///     .insert("active", Value::bool(true))
///     .build();
///
/// assert_eq!(object.len(), 3);
/// ```
#[derive(Debug, Clone)]
pub struct ObjectBuilder {
    map: BTreeMap<String, Value>,
    limits: Option<ValueLimits>,
}

impl Default for ObjectBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectBuilder {
    /// Create a new object builder
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
            limits: None,
        }
    }

    /// Set value limits for validation
    pub fn with_limits(mut self, limits: ValueLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Insert a key-value pair
    pub fn insert(mut self, key: impl Into<String>, value: Value) -> Self {
        self.map.insert(key.into(), value);
        self
    }

    /// Try to insert with limit checking
    pub fn try_insert(mut self, key: impl Into<String>, value: Value) -> Result<Self, ObjectError> {
        let key = key.into();
        if let Some(limits) = &self.limits {
            // Check if we're adding a new key (not replacing)
            if !self.map.contains_key(&key) {
                limits.check_object_keys(self.map.len() + 1)
                    .map_err(|e| ObjectError::InvalidOperation { msg: e.to_string() })?;
            }
        }
        self.map.insert(key, value);
        Ok(self)
    }

    /// Insert multiple key-value pairs
    pub fn extend<I>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = (String, Value)>,
    {
        self.map.extend(iter);
        self
    }

    /// Try to extend with limit checking
    pub fn try_extend<I>(mut self, iter: I) -> Result<Self, ObjectError>
    where
        I: IntoIterator<Item = (String, Value)>,
    {
        if let Some(limits) = &self.limits {
            let iter = iter.into_iter();
            let (lower, _) = iter.size_hint();
            limits.check_object_keys(self.map.len() + lower)
                .map_err(|e| ObjectError::InvalidOperation { msg: e.to_string() })?;
            self.map.extend(iter);
        } else {
            self.map.extend(iter);
        }
        Ok(self)
    }

    /// Remove a key
    pub fn remove(mut self, key: &str) -> Self {
        self.map.remove(key);
        self
    }

    /// Check if key exists
    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Get current number of keys
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Build the final object
    pub fn build(self) -> Object {
        Object::from(self.map)
    }

    /// Try to build with final validation
    pub fn try_build(self) -> Result<Object, ObjectError> {
        if let Some(limits) = &self.limits {
            limits.check_object_keys(self.map.len())
                .map_err(|e| ObjectError::InvalidOperation { msg: e.to_string() })?;
        }
        Ok(Object::from(self.map))
    }
}

impl From<BTreeMap<String, Value>> for ObjectBuilder {
    fn from(map: BTreeMap<String, Value>) -> Self {
        Self {
            map,
            limits: None,
        }
    }
}

impl FromIterator<(String, Value)> for ObjectBuilder {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        Self {
            map: iter.into_iter().collect(),
            limits: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_basic() {
        let object = ObjectBuilder::new()
            .insert("name", Value::string("Alice"))
            .insert("age", Value::int(30))
            .build();

        assert_eq!(object.len(), 2);
        assert_eq!(object.get("name").unwrap().as_str(), Some("Alice"));
    }

    #[test]
    fn test_builder_extend() {
        let pairs = vec![
            ("a".to_string(), Value::int(1)),
            ("b".to_string(), Value::int(2)),
        ];
        let object = ObjectBuilder::new()
            .extend(pairs)
            .build();

        assert_eq!(object.len(), 2);
    }

    #[test]
    fn test_builder_with_limits() {
        let limits = ValueLimits::strict();
        let result = ObjectBuilder::new()
            .with_limits(limits)
            .extend((0..500).map(|i| (format!("key{}", i), Value::int(i as i64))))
            .try_build();

        assert!(result.is_ok());

        // Exceeding limit
        let result = ObjectBuilder::new()
            .with_limits(limits)
            .extend((0..2000).map(|i| (format!("key{}", i), Value::int(i as i64))))
            .try_build();

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_remove() {
        let object = ObjectBuilder::new()
            .insert("a", Value::int(1))
            .insert("b", Value::int(2))
            .remove("a")
            .build();

        assert_eq!(object.len(), 1);
        assert!(!object.contains_key("a"));
    }
}