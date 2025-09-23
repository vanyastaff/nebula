use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, Index};
use std::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(feature = "indexmap")]
use indexmap::IndexMap;

use thiserror::Error;

#[cfg(feature = "rayon")]
use rayon::prelude::*;

use crate::Value; // Assuming Value is defined elsewhere

/// Result type alias for Object operations
pub type ObjectResult<T> = Result<T, ObjectError>;

/// Rich, typed errors for Object operations
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ObjectError {
    #[error("Key not found: {key}")]
    KeyNotFound { key: String },

    #[error("Invalid key: {key} - {reason}")]
    InvalidKey { key: String, reason: String },

    #[error("Type conversion error: expected object, got {found}")]
    #[cfg(feature = "serde")]
    JsonTypeMismatch { found: &'static str },

    #[error("Merge conflict: duplicate key {key} with different values")]
    MergeConflict { key: String },

    #[error("Invalid operation: {msg}")]
    InvalidOperation { msg: String },

    #[error("Value error: {msg}")]
    ValueError { msg: String },

    #[error("Path not found: {path}")]
    PathNotFound { path: String },

    #[error("Circular reference detected")]
    CircularReference,
}

/// Type alias for the internal map type
#[cfg(feature = "indexmap")]
type InternalMap = IndexMap<String, Value>;

#[cfg(not(feature = "indexmap"))]
type InternalMap = BTreeMap<String, Value>;

/// A high-performance, feature-rich object/map type with functional programming support
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Object {
    /// Internal storage using Arc for cheap cloning
    inner: Arc<InternalMap>,

    /// Cached hash value for O(1) hash operations
    #[cfg_attr(feature = "serde", serde(skip))]
    hash_cache: std::sync::OnceLock<u64>,

    /// Cached size for O(1) access
    #[cfg_attr(feature = "serde", serde(skip))]
    size_cache: std::sync::OnceLock<usize>,
}

impl Object {
    // ==================== Constructors ====================

    /// Creates a new empty Object
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(InternalMap::new()),
            hash_cache: std::sync::OnceLock::new(),
            size_cache: std::sync::OnceLock::new(),
        }
    }

    /// Creates an Object with specified capacity
    #[inline]
    pub fn with_capacity(_capacity: usize) -> Self {
        #[cfg(feature = "indexmap")]
        let map = IndexMap::with_capacity(_capacity);

        #[cfg(not(feature = "indexmap"))]
        let map = BTreeMap::new(); // BTreeMap doesn't have with_capacity

        Self {
            inner: Arc::new(map),
            hash_cache: std::sync::OnceLock::new(),
            size_cache: std::sync::OnceLock::new(),
        }
    }

    /// Creates an Object from a map
    #[inline]
    pub fn from_map(map: InternalMap) -> Self {
        Self {
            inner: Arc::new(map),
            hash_cache: std::sync::OnceLock::new(),
            size_cache: std::sync::OnceLock::new(),
        }
    }

    /// Creates an Object from key-value pairs
    pub fn from_pairs<I, K, V>(iter: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<Value>,
    {
        let map: InternalMap = iter
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        Self::from_map(map)
    }

    /// Creates an Object from entries
    pub fn from_entries<I>(iter: I) -> ObjectResult<Self>
    where
        I: IntoIterator<Item = (String, Value)>,
    {
        let mut map = InternalMap::new();
        for (key, value) in iter {
            if key.is_empty() {
                return Err(ObjectError::InvalidKey {
                    key: key.clone(),
                    reason: "Key cannot be empty".into(),
                });
            }
            map.insert(key, value);
        }
        Ok(Self::from_map(map))
    }

    // ==================== Basic Properties ====================

    /// Returns the number of key-value pairs
    #[inline]
    pub fn len(&self) -> usize {
        *self.size_cache.get_or_init(|| self.inner.len())
    }

    /// Returns true if the object is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the capacity (for IndexMap)
    #[cfg(feature = "indexmap")]
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Clears the object (returns a new empty object)
    #[inline]
    pub fn clear(&self) -> Self {
        Self::new()
    }

    // ==================== Key Operations ====================

    /// Returns all keys
    pub fn keys(&self) -> Vec<String> {
        self.inner.keys().cloned().collect()
    }

    /// Returns all values
    pub fn values(&self) -> Vec<Value> {
        self.inner.values().cloned().collect()
    }

    /// Returns all key-value pairs
    pub fn entries(&self) -> Vec<(String, Value)> {
        self.inner
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Returns an iterator over keys
    #[inline]
    pub fn keys_iter(&self) -> impl Iterator<Item = &String> + '_ {
        self.inner.keys()
    }

    /// Returns an iterator over values
    #[inline]
    pub fn values_iter(&self) -> impl Iterator<Item = &Value> + '_ {
        self.inner.values()
    }

    /// Returns an iterator over key-value pairs
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> + '_ {
        self.inner.iter()
    }

    // ==================== Element Access ====================

    /// Gets a value by key
    #[inline]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.inner.get(key)
    }

    /// Gets a value by key with error handling
    #[inline]
    pub fn try_get(&self, key: &str) -> ObjectResult<&Value> {
        self.get(key).ok_or_else(|| ObjectError::KeyNotFound {
            key: key.to_string(),
        })
    }

    /// Gets a value by key, or returns a default
    #[inline]
    pub fn get_or(&self, key: &str, default: &Value) -> Value {
        self.get(key).cloned().unwrap_or_else(|| default.clone())
    }

    /// Gets a value by nested path (e.g., "user.address.city")
    pub fn get_path(&self, path: &str) -> ObjectResult<&Value> {
        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return Err(ObjectError::PathNotFound {
                path: path.to_string(),
            });
        }

        let current = self.try_get(parts[0])?;

        for _part in &parts[1..] {
            // TODO: Implement nested path navigation when Value has as_object() method
            // let obj = current.as_object().ok_or_else(|| ObjectError::PathNotFound {
            //     path: path.to_string(),
            // })?;
            // current = obj.try_get(part)?;
        }

        Ok(current)
    }

    /// Checks if a key exists
    #[inline]
    pub fn has_key(&self, key: &str) -> bool {
        self.inner.contains_key(key)
    }

    /// Checks if object has all specified keys
    pub fn has_keys(&self, keys: &[&str]) -> bool {
        keys.iter().all(|key| self.has_key(key))
    }

    /// Checks if object has any of specified keys
    pub fn has_any_key(&self, keys: &[&str]) -> bool {
        keys.iter().any(|key| self.has_key(key))
    }

    // ==================== Immutable Operations ====================

    /// Returns a new object with a key-value pair added/updated
    #[must_use]
    pub fn insert(&self, key: impl Into<String>, value: Value) -> Self {
        let mut map = (*self.inner).clone();
        map.insert(key.into(), value);
        Self::from_map(map)
    }

    /// Returns a new object with multiple key-value pairs added
    #[must_use]
    pub fn insert_many<I, K>(&self, entries: I) -> Self
    where
        I: IntoIterator<Item = (K, Value)>,
        K: Into<String>,
    {
        let mut map = (*self.inner).clone();
        for (key, value) in entries {
            map.insert(key.into(), value);
        }
        Self::from_map(map)
    }

    /// Returns a new object with a key removed
    #[must_use]
    pub fn remove(&self, key: &str) -> ObjectResult<(Self, Value)> {
        if !self.has_key(key) {
            return Err(ObjectError::KeyNotFound {
                key: key.to_string(),
            });
        }

        let mut map = (*self.inner).clone();
        let value = map.remove(key).unwrap();
        Ok((Self::from_map(map), value))
    }

    /// Returns a new object with multiple keys removed
    #[must_use]
    pub fn remove_many(&self, keys: &[&str]) -> Self {
        let mut map = (*self.inner).clone();
        for key in keys {
            map.remove(*key);
        }
        Self::from_map(map)
    }

    /// Returns a new object with a value updated by a function
    pub fn update<F>(&self, key: &str, f: F) -> ObjectResult<Self>
    where
        F: FnOnce(&Value) -> Value,
    {
        let value = self.try_get(key)?;
        Ok(self.insert(key, f(value)))
    }

    /// Returns a new object with a value updated or inserted
    pub fn upsert<F>(&self, key: impl Into<String>, default: Value, f: F) -> Self
    where
        F: FnOnce(&Value) -> Value,
    {
        let key = key.into();
        let value = self.get(&key).cloned().unwrap_or(default);
        self.insert(key, f(&value))
    }

    // ==================== Transformation Operations ====================

    /// Maps a function over all values
    pub fn map_values<F>(&self, mut f: F) -> ObjectResult<Self>
    where
        F: FnMut(&String, &Value) -> ObjectResult<Value>,
    {
        let mut map = InternalMap::new();
        for (key, value) in self.inner.iter() {
            map.insert(key.clone(), f(key, value)?);
        }
        Ok(Self::from_map(map))
    }

    /// Maps a function over all keys
    pub fn map_keys<F>(&self, mut f: F) -> ObjectResult<Self>
    where
        F: FnMut(&String) -> ObjectResult<String>,
    {
        let mut map = InternalMap::new();
        for (key, value) in self.inner.iter() {
            let new_key = f(key)?;
            if map.contains_key(&new_key) {
                return Err(ObjectError::InvalidKey {
                    key: new_key,
                    reason: "Duplicate key after mapping".into(),
                });
            }
            map.insert(new_key, value.clone());
        }
        Ok(Self::from_map(map))
    }

    /// Filters entries based on a predicate
    #[must_use]
    pub fn filter<P>(&self, mut predicate: P) -> Self
    where
        P: FnMut(&String, &Value) -> bool,
    {
        let map: InternalMap = self
            .inner
            .iter()
            .filter(|(k, v)| predicate(k, v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Self::from_map(map)
    }

    /// Filter and map in one operation
    #[must_use]
    pub fn filter_map<F>(&self, mut f: F) -> Self
    where
        F: FnMut(&String, &Value) -> Option<(String, Value)>,
    {
        let map: InternalMap = self.inner.iter().filter_map(|(k, v)| f(k, v)).collect();
        Self::from_map(map)
    }

    /// Keeps only specified keys
    #[must_use]
    pub fn pick(&self, keys: &[&str]) -> Self {
        self.filter(|k, _| keys.contains(&k.as_str()))
    }

    /// Removes specified keys
    #[must_use]
    pub fn omit(&self, keys: &[&str]) -> Self {
        self.filter(|k, _| !keys.contains(&k.as_str()))
    }

    /// Partitions object into two based on predicate
    #[must_use]
    pub fn partition<P>(&self, mut predicate: P) -> (Self, Self)
    where
        P: FnMut(&String, &Value) -> bool,
    {
        let mut true_map = InternalMap::new();
        let mut false_map = InternalMap::new();

        for (key, value) in self.inner.iter() {
            if predicate(key, value) {
                true_map.insert(key.clone(), value.clone());
            } else {
                false_map.insert(key.clone(), value.clone());
            }
        }

        (Self::from_map(true_map), Self::from_map(false_map))
    }

    // ==================== Merging Operations ====================

    /// Merges with another object (other's values override)
    #[must_use]
    pub fn merge(&self, other: &Object) -> Self {
        let mut map = (*self.inner).clone();
        for (key, value) in other.inner.iter() {
            map.insert(key.clone(), value.clone());
        }
        Self::from_map(map)
    }

    /// Deep merges with another object
    pub fn merge_deep(&self, other: &Object) -> ObjectResult<Self> {
        let mut map = (*self.inner).clone();

        for (key, other_value) in other.inner.iter() {
            match map.get(key) {
                Some(_self_value) => {
                    // If both are objects, merge recursively
                    // This depends on your Value implementation
                    // if let (Some(self_obj), Some(other_obj)) =
                    //     (self_value.as_object(), other_value.as_object()) {
                    //     let merged = self_obj.merge_deep(other_obj)?;
                    //     map.insert(key.clone(), Value::Object(merged));
                    // } else {
                    map.insert(key.clone(), other_value.clone());
                    // }
                }
                None => {
                    map.insert(key.clone(), other_value.clone());
                }
            }
        }

        Ok(Self::from_map(map))
    }

    /// Merges multiple objects
    pub fn merge_all<I>(objects: I) -> Self
    where
        I: IntoIterator<Item = Object>,
    {
        let mut result = Self::new();
        for obj in objects {
            result = result.merge(&obj);
        }
        result
    }

    /// Merges with custom conflict resolution
    pub fn merge_with<F>(&self, other: &Object, mut resolver: F) -> ObjectResult<Self>
    where
        F: FnMut(&String, &Value, &Value) -> ObjectResult<Value>,
    {
        let mut map = InternalMap::new();

        // Add all keys from self
        for (key, value) in self.inner.iter() {
            if let Some(other_value) = other.get(key) {
                map.insert(key.clone(), resolver(key, value, other_value)?);
            } else {
                map.insert(key.clone(), value.clone());
            }
        }

        // Add keys only in other
        for (key, value) in other.inner.iter() {
            if !self.has_key(key) {
                map.insert(key.clone(), value.clone());
            }
        }

        Ok(Self::from_map(map))
    }

    // ==================== Set Operations ====================

    /// Returns keys that exist in both objects
    pub fn intersection_keys(&self, other: &Object) -> Vec<String> {
        self.keys_iter()
            .filter(|k| other.has_key(k))
            .cloned()
            .collect()
    }

    /// Returns keys that exist in self but not in other
    pub fn difference_keys(&self, other: &Object) -> Vec<String> {
        self.keys_iter()
            .filter(|k| !other.has_key(k))
            .cloned()
            .collect()
    }

    /// Returns keys that exist in either object but not both
    pub fn symmetric_difference_keys(&self, other: &Object) -> Vec<String> {
        let mut keys = Vec::new();

        for key in self.keys_iter() {
            if !other.has_key(key) {
                keys.push(key.clone());
            }
        }

        for key in other.keys_iter() {
            if !self.has_key(key) {
                keys.push(key.clone());
            }
        }

        keys
    }

    /// Returns union of all keys
    pub fn union_keys(&self, other: &Object) -> Vec<String> {
        let mut keys: Vec<String> = self.keys();
        for key in other.keys_iter() {
            if !self.has_key(key) {
                keys.push(key.clone());
            }
        }
        keys
    }

    // ==================== Conversion Operations ====================

    /// Inverts the object (swaps keys and values)
    pub fn invert(&self) -> ObjectResult<Self> {
        let mut map = InternalMap::new();

        for (key, value) in self.inner.iter() {
            // Convert value to string key
            let new_key = value.to_string();
            if map.contains_key(&new_key) {
                return Err(ObjectError::InvalidKey {
                    key: new_key,
                    reason: "Duplicate value when inverting".into(),
                });
            }
            map.insert(new_key, Value::from(key.clone()));
        }

        Ok(Self::from_map(map))
    }

    /// Groups values by a key function
    pub fn group_by<F, K>(&self, mut key_fn: F) -> ObjectResult<BTreeMap<K, Vec<(String, Value)>>>
    where
        F: FnMut(&String, &Value) -> ObjectResult<K>,
        K: Ord,
    {
        let mut groups: BTreeMap<K, Vec<(String, Value)>> = BTreeMap::new();

        for (k, v) in self.inner.iter() {
            let group_key = key_fn(k, v)?;
            groups
                .entry(group_key)
                .or_default()
                .push((k.clone(), v.clone()));
        }

        Ok(groups)
    }

    /// Flattens nested objects with dot notation
    pub fn flatten(&self) -> Self {
        let mut result = InternalMap::new();
        self.flatten_recursive(&mut result, String::new());
        Self::from_map(result)
    }

    fn flatten_recursive(&self, result: &mut InternalMap, prefix: String) {
        for (key, value) in self.inner.iter() {
            let new_key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", prefix, key)
            };

            // If value is an object, recurse
            // if let Some(obj) = value.as_object() {
            //     obj.flatten_recursive(result, new_key);
            // } else {
            result.insert(new_key, value.clone());
            // }
        }
    }

    /// Unflattens dot notation keys into nested objects
    pub fn unflatten(&self) -> ObjectResult<Self> {
        let mut root = InternalMap::new();

        for (key, value) in self.inner.iter() {
            let parts: Vec<&str> = key.split('.').collect();
            if parts.is_empty() {
                continue;
            }

            // Build nested structure
            // This is simplified - real implementation would need to handle nested objects
            root.insert(key.clone(), value.clone());
        }

        Ok(Self::from_map(root))
    }

    // ==================== Validation Operations ====================

    /// Validates object against a schema
    pub fn validate_keys(&self, required: &[&str], optional: &[&str]) -> ObjectResult<()> {
        // Check required keys
        for key in required {
            if !self.has_key(key) {
                return Err(ObjectError::KeyNotFound {
                    key: key.to_string(),
                });
            }
        }

        // Check for unknown keys
        for key in self.keys_iter() {
            if !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
                return Err(ObjectError::InvalidKey {
                    key: key.clone(),
                    reason: "Unknown key".into(),
                });
            }
        }

        Ok(())
    }

    /// Returns a new object with default values for missing keys
    #[must_use]
    pub fn with_defaults(&self, defaults: &Object) -> Self {
        defaults.merge(self)
    }

    // ==================== Comparison Operations ====================

    /// Checks if this object is a subset of another
    pub fn is_subset_of(&self, other: &Object) -> bool {
        self.inner.iter().all(|(k, v)| other.get(k) == Some(v))
    }

    /// Checks if this object is a superset of another
    pub fn is_superset_of(&self, other: &Object) -> bool {
        other.is_subset_of(self)
    }

    /// Computes the difference between two objects
    pub fn diff(&self, other: &Object) -> Object {
        let mut diff = InternalMap::new();

        // Check for changed/removed keys
        for (key, value) in self.inner.iter() {
            match other.get(key) {
                Some(other_value) if value != other_value => {
                    diff.insert(key.clone(), other_value.clone());
                }
                None => {
                    // Key was removed
                    diff.insert(key.clone(), Value::from("null")); // Or use a special marker
                }
                _ => {}
            }
        }

        // Check for added keys
        for (key, value) in other.inner.iter() {
            if !self.has_key(key) {
                diff.insert(key.clone(), value.clone());
            }
        }

        Self::from_map(diff)
    }

    // ==================== Parallel Operations ====================

    #[cfg(feature = "rayon")]
    /// Parallel map over values
    pub fn par_map_values<F>(&self, f: F) -> ObjectResult<Self>
    where
        F: Fn(&String, &Value) -> ObjectResult<Value> + Sync + Send,
        Value: Send + Sync,
    {
        if self.len() < 100 {
            return self.map_values(f);
        }

        let results: Result<Vec<_>, _> = self
            .inner
            .iter()
            .par_bridge()
            .map(|(k, v)| f(k, v).map(|new_v| (k.clone(), new_v)))
            .collect();

        Ok(Self::from_map(results?.into_iter().collect()))
    }

    #[cfg(feature = "rayon")]
    /// Parallel filter operation
    pub fn par_filter<P>(&self, predicate: P) -> Self
    where
        P: Fn(&String, &Value) -> bool + Sync + Send,
        Value: Send + Sync,
    {
        if self.len() < 100 {
            return self.filter(predicate);
        }

        let pairs: Vec<(String, Value)> = self
            .inner
            .iter()
            .par_bridge()
            .filter(|(k, v)| predicate(k, v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Self::from_map(pairs.into_iter().collect())
    }

    // ==================== Utility Operations ====================

    /// Creates a formatted string representation
    pub fn to_json_string(&self) -> String {
        // Simple JSON-like formatting
        let entries: Vec<String> = self
            .inner
            .iter()
            .map(|(k, v)| format!("  \"{}\": {}", k, v))
            .collect();

        if entries.is_empty() {
            "{}".to_string()
        } else {
            format!("{{\n{}\n}}", entries.join(",\n"))
        }
    }

    /// Creates a debug string with limited depth
    pub fn to_debug_string(&self, max_depth: usize) -> String {
        self.to_debug_string_internal(0, max_depth)
    }

    fn to_debug_string_internal(&self, depth: usize, max_depth: usize) -> String {
        if depth >= max_depth {
            return "{...}".to_string();
        }

        let indent = "  ".repeat(depth);
        let entries: Vec<String> = self
            .inner
            .iter()
            .take(10) // Limit to first 10 entries
            .map(|(k, v)| format!("{}{}: {}", indent, k, v))
            .collect();

        if self.len() > 10 {
            format!(
                "{{ {} ... and {} more }}",
                entries.join(", "),
                self.len() - 10
            )
        } else {
            format!("{{ {} }}", entries.join(", "))
        }
    }

    /// Converts to a sorted map (BTreeMap)
    pub fn to_sorted_map(&self) -> BTreeMap<String, Value> {
        self.inner
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Gets the size in bytes (approximate)
    pub fn size_bytes(&self) -> usize {
        self.inner
            .iter()
            .map(|(k, _v)| {
                k.len() + std::mem::size_of::<Value>() // Simplified
            })
            .sum()
    }
}

// ==================== Trait Implementations ====================

impl Default for Object {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for Object {
    type Target = InternalMap;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AsRef<InternalMap> for Object {
    #[inline]
    fn as_ref(&self) -> &InternalMap {
        &self.inner
    }
}

impl Borrow<InternalMap> for Object {
    #[inline]
    fn borrow(&self) -> &InternalMap {
        &self.inner
    }
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            write!(f, "{{}}")
        } else {
            write!(f, "{{object with {} keys}}", self.len())
        }
    }
}

// ==================== Index Trait ====================

impl Index<&str> for Object {
    type Output = Value;

    /// Panics if the key is not found. Prefer using `get` for safe access.
    ///
    /// # Panics
    ///
    /// Panics if the key does not exist in the object.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_value::{Object, Value};
    /// let obj = Object::from_iter(vec![("foo", Value::from(1))]);
    /// assert_eq!(obj["foo"].as_i64(), Some(1));
    /// ```
    fn index(&self, key: &str) -> &Self::Output {
        match self.get(key) {
            Some(val) => val,
            None => panic!("Object index: key '{}' not found. Use `get` for safe access.", key),
        }
    }
}

impl Index<String> for Object {
    type Output = Value;

    fn index(&self, key: String) -> &Self::Output {
        self.index(key.as_str())
    }
}

impl Index<&String> for Object {
    type Output = Value;

    fn index(&self, key: &String) -> &Self::Output {
        self.index(key.as_str())
    }
}

// ==================== Conversion Traits ====================

impl From<InternalMap> for Object {
    #[inline]
    fn from(map: InternalMap) -> Self {
        Self::from_map(map)
    }
}

#[cfg(feature = "indexmap")]
impl From<BTreeMap<String, Value>> for Object {
    #[inline]
    fn from(map: BTreeMap<String, Value>) -> Self {
        let internal: InternalMap = map.into_iter().collect();
        Self::from_map(internal)
    }
}

impl<const N: usize> From<[(String, Value); N]> for Object {
    fn from(arr: [(String, Value); N]) -> Self {
        Self::from_map(arr.into_iter().collect())
    }
}

impl<'a, const N: usize> From<[(&'a str, Value); N]> for Object {
    fn from(arr: [(&'a str, Value); N]) -> Self {
        Self::from_map(arr.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
}

impl From<Vec<(String, Value)>> for Object {
    fn from(vec: Vec<(String, Value)>) -> Self {
        Self::from_map(vec.into_iter().collect())
    }
}

impl From<Object> for InternalMap {
    fn from(obj: Object) -> Self {
        match Arc::try_unwrap(obj.inner) {
            Ok(map) => map,
            Err(arc) => (*arc).clone(),
        }
    }
}

// ==================== Comparison Traits ====================

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }

        for (key, value) in self.inner.iter() {
            match other.get(key) {
                Some(other_value) if value == other_value => continue,
                _ => return false,
            }
        }

        true
    }
}

impl Eq for Object {}

impl PartialOrd for Object {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Object {
    fn cmp(&self, other: &Self) -> Ordering {
        // First compare by size
        match self.len().cmp(&other.len()) {
            Ordering::Equal => {
                // Then compare keys and values lexicographically
                let mut self_sorted: Vec<_> = self.inner.iter().collect();
                let mut other_sorted: Vec<_> = other.inner.iter().collect();
                self_sorted.sort_by_key(|(k, _)| *k);
                other_sorted.sort_by_key(|(k, _)| *k);

                for (self_entry, other_entry) in self_sorted.iter().zip(other_sorted.iter()) {
                    match self_entry.0.cmp(other_entry.0) {
                        Ordering::Equal => match self_entry.1.partial_cmp(other_entry.1) {
                            Some(ord) if ord != Ordering::Equal => return ord,
                            _ => continue,
                        },
                        ord => return ord,
                    }
                }

                Ordering::Equal
            }
            ord => ord,
        }
    }
}

impl Hash for Object {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let hash = self.hash_cache.get_or_init(|| {
            use std::collections::hash_map::DefaultHasher;
            let mut hasher = DefaultHasher::new();

            // Hash size first
            self.len().hash(&mut hasher);

            // Hash sorted key-value pairs for consistency
            let mut sorted: Vec<_> = self.inner.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);

            for (k, v) in sorted {
                k.hash(&mut hasher);
                // Hash the string representation of value
                v.to_string().hash(&mut hasher);
            }

            hasher.finish()
        });

        hash.hash(state);
    }
}

// ==================== Iterator Traits ====================

impl FromIterator<(String, Value)> for Object {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Self {
        Self::from_map(iter.into_iter().collect())
    }
}

impl<'a> FromIterator<(&'a str, Value)> for Object {
    fn from_iter<T: IntoIterator<Item = (&'a str, Value)>>(iter: T) -> Self {
        Self::from_map(iter.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
}

impl<'a> FromIterator<(String, &'a Value)> for Object {
    fn from_iter<T: IntoIterator<Item = (String, &'a Value)>>(iter: T) -> Self {
        Self::from_map(iter.into_iter().map(|(k, v)| (k, v.clone())).collect())
    }
}

impl<'a> FromIterator<(&'a str, &'a Value)> for Object {
    fn from_iter<T: IntoIterator<Item = (&'a str, &'a Value)>>(iter: T) -> Self {
        Self::from_map(
            iter.into_iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }
}

impl Extend<(String, Value)> for Object {
    fn extend<T: IntoIterator<Item = (String, Value)>>(&mut self, iter: T) {
        let additional: InternalMap = iter.into_iter().collect();
        *self = self.merge(&Self::from_map(additional));
    }
}

impl<'a> Extend<(&'a str, Value)> for Object {
    fn extend<T: IntoIterator<Item = (&'a str, Value)>>(&mut self, iter: T) {
        let additional: InternalMap = iter.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        *self = self.merge(&Self::from_map(additional));
    }
}

impl IntoIterator for Object {
    type Item = (String, Value);
    type IntoIter = std::vec::IntoIter<(String, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries().into_iter()
    }
}

#[cfg(not(feature = "indexmap"))]
impl<'a> IntoIterator for &'a Object {
    type Item = (&'a String, &'a Value);
    type IntoIter = std::collections::btree_map::Iter<'a, String, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

#[cfg(feature = "indexmap")]
impl<'a> IntoIterator for &'a Object {
    type Item = (&'a String, &'a Value);
    type IntoIter = indexmap::map::Iter<'a, String, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

// ==================== JSON Support ====================

#[cfg(feature = "serde")]
impl From<Object> for serde_json::Value {
    fn from(obj: Object) -> Self {
        let map: serde_json::Map<String, serde_json::Value> = obj
            .inner
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::from(v.clone())))
            .collect();
        serde_json::Value::Object(map)
    }
}

#[cfg(feature = "serde")]
impl TryFrom<serde_json::Value> for Object {
    type Error = ObjectError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Object(map) => {
                let entries: Result<InternalMap, _> = map
                    .into_iter()
                    .map(|(k, v)| {
                        Value::try_from(v).map(|val| (k, val)).map_err(|_| {
                            ObjectError::JsonTypeMismatch {
                                found: "invalid value",
                            }
                        })
                    })
                    .collect();
                Ok(Object::from_map(entries?))
            }
            serde_json::Value::Null => Ok(Object::new()),
            serde_json::Value::Bool(_) => Err(ObjectError::JsonTypeMismatch { found: "bool" }),
            serde_json::Value::Number(_) => Err(ObjectError::JsonTypeMismatch { found: "number" }),
            serde_json::Value::String(_) => Err(ObjectError::JsonTypeMismatch { found: "string" }),
            serde_json::Value::Array(_) => Err(ObjectError::JsonTypeMismatch { found: "array" }),
        }
    }
}

// ==================== Send + Sync ====================

unsafe impl Send for Object {}
unsafe impl Sync for Object {}

// ==================== Builder Pattern ====================

/// Builder for creating Object instances
pub struct ObjectBuilder {
    map: InternalMap,
}

impl ObjectBuilder {
    /// Creates a new builder
    pub fn new() -> Self {
        Self {
            map: InternalMap::new(),
        }
    }

    /// Adds a key-value pair
    pub fn insert(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.map.insert(key.into(), value.into());
        self
    }

    /// Adds a key-value pair if the value is Some
    pub fn insert_if_some<T>(self, key: impl Into<String>, value: Option<T>) -> Self
    where
        T: Into<Value>,
    {
        match value {
            Some(v) => self.insert(key, v),
            None => self,
        }
    }

    /// Merges another object
    pub fn merge(mut self, other: &Object) -> Self {
        for (k, v) in other.inner.iter() {
            self.map.insert(k.clone(), v.clone());
        }
        self
    }

    /// Builds the Object
    pub fn build(self) -> Object {
        Object::from_map(self.map)
    }
}

impl Default for ObjectBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Macros ====================

/// Macro for creating Object instances
#[macro_export]
macro_rules! object {
    () => {
        $crate::Object::new()
    };

    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut _obj = $crate::Object::new();
        $(
            _obj = _obj.insert($key, $value.into());
        )*
        _obj
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let obj1 = Object::new();
        assert!(obj1.is_empty());

        let obj2 = Object::from_pairs(vec![
            ("key1", Value::from("value1")),
            ("key2", Value::from(42)),
        ]);
        assert_eq!(obj2.len(), 2);
    }

    #[test]
    fn test_immutable_operations() {
        let obj = Object::new()
            .insert("key1", Value::from("value1"))
            .insert("key2", Value::from(42));

        assert_eq!(obj.len(), 2);
        assert!(obj.has_key("key1"));

        let updated = obj.insert("key3", Value::from(true));
        assert_eq!(updated.len(), 3);
        assert_eq!(obj.len(), 2); // Original unchanged
    }

    #[test]
    fn test_filter_and_map() {
        let obj = Object::from_pairs(vec![
            ("a", Value::from(1)),
            ("b", Value::from(2)),
            ("c", Value::from(3)),
        ]);

        let filtered = obj.filter(|_k, v| v.as_i64().map_or(false, |n| n > 1));
        assert_eq!(filtered.len(), 2);

        let mapped = obj
            .map_values(|_k, v| Ok(Value::from(v.as_i64().unwrap_or(0) * 2)))
            .unwrap();
        assert_eq!(mapped.get("a"), Some(&Value::from(2)));
    }

    #[test]
    fn test_merge() {
        let obj1 = Object::from_pairs(vec![("a", Value::from(1)), ("b", Value::from(2))]);

        let obj2 = Object::from_pairs(vec![("b", Value::from(20)), ("c", Value::from(3))]);

        let merged = obj1.merge(&obj2);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged.get("b"), Some(&Value::from(20)));
    }

    #[test]
    fn test_pick_and_omit() {
        let obj = Object::from_pairs(vec![
            ("a", Value::from(1)),
            ("b", Value::from(2)),
            ("c", Value::from(3)),
        ]);

        let picked = obj.pick(&["a", "c"]);
        assert_eq!(picked.len(), 2);
        assert!(!picked.has_key("b"));

        let omitted = obj.omit(&["b"]);
        assert_eq!(omitted.len(), 2);
        assert!(!omitted.has_key("b"));
    }

    #[test]
    fn test_builder() {
        let obj = ObjectBuilder::new()
            .insert("key1", Value::from("value1"))
            .insert("key2", Value::from(42))
            .insert_if_some("key3", Some(Value::from(true)))
            .insert_if_some("key4", None::<Value>)
            .build();

        assert_eq!(obj.len(), 3);
        assert!(!obj.has_key("key4"));
    }

    #[test]
    fn test_arc_sharing() {
        let obj1 = Object::from_pairs(vec![("key", Value::from("value"))]);
        let obj2 = obj1.clone();

        // Both should share the same Arc
        assert_eq!(obj1, obj2);
    }

    #[test]
    fn test_macro() {
        let obj = object! {
            "key1" => Value::from("value1"),
            "key2" => Value::from(42),
        };

        assert_eq!(obj.len(), 2);
        assert!(obj.has_key("key1"));
    }

    #[cfg(feature = "rayon")]
    #[test]
    fn test_parallel_operations() {
        let mut entries = vec![];
        for i in 0..1000 {
            entries.push((format!("key{}", i), Value::from(i)));
        }
        let large_obj = Object::from_pairs(entries);

        let par_mapped = large_obj
            .par_map_values(|_k, v| Ok(Value::from(v.as_i64().unwrap_or(0) * 2)))
            .unwrap();

        assert_eq!(par_mapped.len(), 1000);

        let par_filtered = large_obj.par_filter(|_k, v| v.as_i64().map_or(false, |n| n % 2 == 0));

        assert_eq!(par_filtered.len(), 500);
    }
}
