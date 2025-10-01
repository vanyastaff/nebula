//!
//! - Key count limits for DoS protection
//! - O(log n) operations
//! - Thread-safe via Arc
//! - Uses persistent data structures (im::HashMap) for efficient cloning
//! Object (key-value map) type for nebula-value
//! This module provides an Object type that:
pub mod builder;

pub use builder::ObjectBuilder;

use std::fmt;
use std::hash::{Hash, Hasher};

use im::HashMap;

use crate::core::error::{ValueErrorExt, ValueResult};
use crate::core::limits::ValueLimits;
use crate::core::NebulaError;

// Forward declaration - will be replaced with actual Value type
type ValueItem = serde_json::Value;

/// Persistent key-value map with efficient structural sharing
///
/// Uses im::HashMap internally which provides:
/// - O(log n) get/insert/remove
/// - Efficient cloning via structural sharing
/// - Thread-safe immutable operations
#[derive(Debug, Clone)]
pub struct Object {
    inner: HashMap<String, ValueItem>,
}

impl Object {
    /// Create an empty object
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    /// Create from iterator of key-value pairs
    pub fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (String, ValueItem)>,
    {
        Self {
            inner: iter.into_iter().collect(),
        }
    }

    /// Create with key count validation
    pub fn with_limits<I>(iter: I, limits: &ValueLimits) -> ValueResult<Self>
    where
        I: IntoIterator<Item = (String, ValueItem)>,
    {
        let map: HashMap<String, ValueItem> = iter.into_iter().collect();
        limits.check_object_keys(map.len())?;
        Ok(Self { inner: map })
    }

    /// Get the number of keys
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get value by key
    pub fn get(&self, key: &str) -> Option<&ValueItem> {
        self.inner.get(key)
    }

    /// Get value by key or error
    pub fn try_get(&self, key: &str) -> ValueResult<&ValueItem> {
        self.get(key)
            .ok_or_else(|| NebulaError::value_key_not_found(key))
    }

    /// Check if key exists
    pub fn contains_key(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    /// Insert key-value pair (returns new Object, original unchanged)
    pub fn insert(&self, key: String, value: ValueItem) -> Self {
        let mut new_map = self.inner.clone();
        new_map.insert(key, value);
        Self { inner: new_map }
    }

    /// Insert with limit check
    pub fn insert_with_limit(
        &self,
        key: String,
        value: ValueItem,
        limits: &ValueLimits,
    ) -> ValueResult<Self> {
        let new_size = if self.contains_key(&key) {
            self.len()
        } else {
            self.len() + 1
        };
        limits.check_object_keys(new_size)?;
        Ok(self.insert(key, value))
    }

    /// Remove key (returns new Object and removed value)
    pub fn remove(&self, key: &str) -> Option<(Self, ValueItem)> {
        let mut new_map = self.inner.clone();
        new_map.remove(key).map(|val| {
            (Self { inner: new_map }, val)
        })
    }

    /// Get all keys
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.inner.keys()
    }

    /// Get all values
    pub fn values(&self) -> impl Iterator<Item = &ValueItem> {
        self.inner.values()
    }

    /// Get all entries
    pub fn entries(&self) -> impl Iterator<Item = (&String, &ValueItem)> {
        self.inner.iter()
    }

    /// Merge with another object (right wins on conflicts)
    pub fn merge(&self, other: &Object) -> Self {
        let mut new_map = self.inner.clone();
        for (k, v) in other.inner.iter() {
            new_map.insert(k.clone(), v.clone());
        }
        Self { inner: new_map }
    }

    /// Merge with limit check
    pub fn merge_with_limit(&self, other: &Object, limits: &ValueLimits) -> ValueResult<Self> {
        // Calculate potential size (worst case: no overlapping keys)
        let potential_size = self.len() + other.len();
        limits.check_object_keys(potential_size)?;
        Ok(self.merge(other))
    }

    /// Filter entries by predicate
    pub fn filter<F>(&self, predicate: F) -> Self
    where
        F: Fn(&String, &ValueItem) -> bool,
    {
        let filtered: HashMap<String, ValueItem> = self.inner
            .iter()
            .filter(|(k, v)| predicate(k, v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Self { inner: filtered }
    }

    /// Convert to Vec of tuples
    pub fn to_vec(&self) -> Vec<(String, ValueItem)> {
        self.inner
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

impl Default for Object {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Object {}

impl Hash for Object {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash keys in sorted order for deterministic hashing
        let mut keys: Vec<_> = self.inner.keys().collect();
        keys.sort();
        for key in keys {
            key.hash(state);
            if let Some(value) = self.inner.get(key) {
                format!("{:?}", value).hash(state);
            }
        }
    }
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{} keys}}", self.len())
    }
}

impl FromIterator<(String, ValueItem)> for Object {
    fn from_iter<I: IntoIterator<Item = (String, ValueItem)>>(iter: I) -> Self {
        Self::from_iter(iter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_object_creation() {
        let obj = Object::new();
        assert_eq!(obj.len(), 0);
        assert!(obj.is_empty());
    }

    #[test]
    fn test_object_from_iter() {
        let obj = Object::from_iter(vec![
            ("name".to_string(), json!("Alice")),
            ("age".to_string(), json!(30)),
        ]);

        assert_eq!(obj.len(), 2);
        assert_eq!(obj.get("name"), Some(&json!("Alice")));
        assert_eq!(obj.get("age"), Some(&json!(30)));
    }

    #[test]
    fn test_object_insert() {
        let obj = Object::new();
        let obj = obj.insert("key1".to_string(), json!(1));
        let obj = obj.insert("key2".to_string(), json!(2));

        assert_eq!(obj.len(), 2);
        assert_eq!(obj.get("key1"), Some(&json!(1)));
        assert_eq!(obj.get("key2"), Some(&json!(2)));
    }

    #[test]
    fn test_object_remove() {
        let obj = Object::from_iter(vec![
            ("key1".to_string(), json!(1)),
            ("key2".to_string(), json!(2)),
        ]);

        let (obj, removed) = obj.remove("key1").unwrap();
        assert_eq!(removed, json!(1));
        assert_eq!(obj.len(), 1);
        assert!(!obj.contains_key("key1"));
        assert!(obj.contains_key("key2"));
    }

    #[test]
    fn test_object_structural_sharing() {
        let obj1 = Object::from_iter(vec![
            ("key1".to_string(), json!(1)),
        ]);
        let obj2 = obj1.insert("key2".to_string(), json!(2));

        assert_eq!(obj1.len(), 1);
        assert_eq!(obj2.len(), 2);
    }

    #[test]
    fn test_object_merge() {
        let obj1 = Object::from_iter(vec![
            ("a".to_string(), json!(1)),
            ("b".to_string(), json!(2)),
        ]);
        let obj2 = Object::from_iter(vec![
            ("b".to_string(), json!(99)),
            ("c".to_string(), json!(3)),
        ]);

        let merged = obj1.merge(&obj2);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged.get("a"), Some(&json!(1)));
        assert_eq!(merged.get("b"), Some(&json!(99))); // obj2 wins
        assert_eq!(merged.get("c"), Some(&json!(3)));
    }

    #[test]
    fn test_object_keys_values() {
        let obj = Object::from_iter(vec![
            ("name".to_string(), json!("Alice")),
            ("age".to_string(), json!(30)),
        ]);

        let keys: Vec<_> = obj.keys().cloned().collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"name".to_string()));
        assert!(keys.contains(&"age".to_string()));

        let values: Vec<_> = obj.values().cloned().collect();
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_object_filter() {
        let obj = Object::from_iter(vec![
            ("a".to_string(), json!(1)),
            ("b".to_string(), json!(2)),
            ("c".to_string(), json!(3)),
        ]);

        let filtered = obj.filter(|_k, v| v.as_i64().unwrap() > 1);
        assert_eq!(filtered.len(), 2);
        assert!(!filtered.contains_key("a"));
        assert!(filtered.contains_key("b"));
    }

    #[test]
    fn test_object_equality() {
        let obj1 = Object::from_iter(vec![
            ("a".to_string(), json!(1)),
        ]);
        let obj2 = Object::from_iter(vec![
            ("a".to_string(), json!(1)),
        ]);
        let obj3 = Object::from_iter(vec![
            ("a".to_string(), json!(2)),
        ]);

        assert_eq!(obj1, obj2);
        assert_ne!(obj1, obj3);
    }
}
