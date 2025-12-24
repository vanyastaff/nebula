//! Runtime parameter values storage
//!
//! This module provides type-safe storage for parameter values at runtime,
//! separate from parameter definitions/schemas.

use crate::core::ParameterError;
use nebula_core::ParameterKey;
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::{Index, IndexMut};

// =============================================================================
// Core ParameterValues
// =============================================================================

/// Runtime storage for parameter values
///
/// This structure stores only the values of parameters, not their schemas.
/// It's designed to be lightweight and efficient for runtime use.
///
/// # Design Philosophy
///
/// - **Separation of Concerns**: Values are stored separately from definitions
/// - **Memory Efficient**: No duplication of metadata, validation rules, etc.
/// - **Type-Safe Access**: Strong typing through helper methods
/// - **Standard Rust Patterns**: Implements `FromIterator`, `Extend`, `Index`, etc.
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::ParameterValues;
/// use nebula_value::Value;
/// use nebula_core::ParameterKey;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut values = ParameterValues::new();
///
/// // Set values
/// values.set(ParameterKey::new("username")?, Value::text("alice"));
/// values.set(ParameterKey::new("age")?, Value::integer(30));
///
/// // Get values
/// let username: String = values.get_typed(ParameterKey::new("username")?)?;
/// assert_eq!(username, "alice");
///
/// // Batch operations
/// values.extend([
///     (ParameterKey::new("email")?, Value::text("alice@example.com")),
///     (ParameterKey::new("verified")?, Value::boolean(true)),
/// ]);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterValues {
    /// Internal storage for values
    values: HashMap<ParameterKey, Value>,
}

impl ParameterValues {
    /// Create a new empty parameter values collection
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with initial capacity
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: HashMap::with_capacity(capacity),
        }
    }

    // =========================================================================
    // Basic Operations
    // =========================================================================

    /// Get a value by key (immutable)
    #[must_use]
    pub fn get(&self, key: impl Into<ParameterKey>) -> Option<&Value> {
        self.values.get(&key.into())
    }

    /// Get a value by key (mutable)
    pub fn get_mut(&mut self, key: impl Into<ParameterKey>) -> Option<&mut Value> {
        self.values.get_mut(&key.into())
    }

    /// Set a value for a parameter
    ///
    /// This is the basic setter without validation. For validated setting,
    /// use `ParameterContext::set()` which validates against the schema.
    pub fn set(&mut self, key: impl Into<ParameterKey>, value: Value) {
        self.values.insert(key.into(), value);
    }

    /// Remove a value and return it
    pub fn remove(&mut self, key: impl Into<ParameterKey>) -> Option<Value> {
        self.values.remove(&key.into())
    }

    /// Check if a parameter has a value
    #[must_use]
    pub fn contains(&self, key: impl Into<ParameterKey>) -> bool {
        self.values.contains_key(&key.into())
    }

    /// Clear all values
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Get the number of values
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    // =========================================================================
    // Type-Safe Access
    // =========================================================================

    /// Get a typed value with automatic conversion
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The key doesn't exist
    /// - The value cannot be converted to the requested type
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nebula_parameter::ParameterValues;
    /// # use nebula_value::Value;
    /// # use nebula_core::ParameterKey;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut values = ParameterValues::new();
    /// values.set(ParameterKey::new("count")?, Value::integer(42));
    ///
    /// let count: i32 = values.get_typed(ParameterKey::new("count")?)?;
    /// assert_eq!(count, 42);
    ///
    /// // Type mismatch error
    /// let result: Result<String, _> = values.get_typed(ParameterKey::new("count")?);
    /// assert!(result.is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_typed<T>(&self, key: impl Into<ParameterKey>) -> Result<T, ParameterError>
    where
        T: TryFrom<Value>,
        T::Error: std::fmt::Display,
    {
        let key = key.into();
        let value = self
            .get(key.clone())
            .ok_or_else(|| ParameterError::NotFound { key: key.clone() })?;

        value
            .clone()
            .try_into()
            .map_err(|_e: T::Error| ParameterError::InvalidType {
                key,
                expected_type: std::any::type_name::<T>().to_string(),
                actual_details: format!("{}", value.kind()),
            })
    }

    /// Get a typed value or return a default
    ///
    /// Unlike `get_typed`, this doesn't error if the key is missing.
    #[must_use]
    pub fn get_typed_or<T>(&self, key: impl Into<ParameterKey>, default: T) -> T
    where
        T: TryFrom<Value>,
        T::Error: std::fmt::Display,
    {
        self.get_typed(key).unwrap_or(default)
    }

    /// Get a typed value or compute it lazily
    pub fn get_typed_or_else<T, F>(&self, key: impl Into<ParameterKey>, f: F) -> T
    where
        T: TryFrom<Value>,
        T::Error: std::fmt::Display,
        F: FnOnce() -> T,
    {
        self.get_typed(key).unwrap_or_else(|_| f())
    }

    // =========================================================================
    // Convenience Getters for Common Types
    // =========================================================================

    /// Get a string value
    pub fn get_string(&self, key: impl Into<ParameterKey>) -> Result<String, ParameterError> {
        self.get_typed(key)
    }

    /// Get a number value
    pub fn get_number(&self, key: impl Into<ParameterKey>) -> Result<f64, ParameterError> {
        self.get_typed(key)
    }

    /// Get an integer value
    pub fn get_int(&self, key: impl Into<ParameterKey>) -> Result<i64, ParameterError> {
        self.get_typed(key)
    }

    /// Get a boolean value
    pub fn get_bool(&self, key: impl Into<ParameterKey>) -> Result<bool, ParameterError> {
        self.get_typed(key)
    }

    /// Get an array value
    pub fn get_array(&self, key: impl Into<ParameterKey>) -> Result<Vec<Value>, ParameterError> {
        self.get_typed(key)
    }

    /// Get an object value
    pub fn get_object(
        &self,
        key: impl Into<ParameterKey>,
    ) -> Result<nebula_value::Object, ParameterError> {
        self.get_typed(key)
    }

    // =========================================================================
    // Batch Operations
    // =========================================================================

    /// Set multiple values at once
    ///
    /// This is more efficient than calling `set()` multiple times.
    pub fn set_many(&mut self, values: impl IntoIterator<Item = (ParameterKey, Value)>) {
        self.values.extend(values);
    }

    /// Try to set multiple values, collecting errors
    ///
    /// Unlike `set_many`, this validates each value and returns all errors.
    /// Useful when you want to report all validation failures at once.
    pub fn try_set_many<F>(
        &mut self,
        values: impl IntoIterator<Item = (ParameterKey, Value)>,
        mut validator: F,
    ) -> Result<(), Vec<(ParameterKey, ParameterError)>>
    where
        F: FnMut(&ParameterKey, &Value) -> Result<(), ParameterError>,
    {
        let mut errors = Vec::new();
        let mut valid_values = Vec::new();

        for (key, value) in values {
            match validator(&key, &value) {
                Ok(()) => valid_values.push((key, value)),
                Err(e) => errors.push((key, e)),
            }
        }

        if errors.is_empty() {
            self.values.extend(valid_values);
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Remove multiple values at once
    pub fn remove_many(&mut self, keys: impl IntoIterator<Item = ParameterKey>) {
        for key in keys {
            self.values.remove(&key);
        }
    }

    /// Retain only values matching a predicate
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&ParameterKey, &Value) -> bool,
    {
        self.values.retain(|k, v| f(k, v));
    }

    // =========================================================================
    // Iteration
    // =========================================================================

    /// Iterate over all key-value pairs
    pub fn iter(&self) -> impl Iterator<Item = (&ParameterKey, &Value)> {
        self.values.iter()
    }

    /// Iterate over all key-value pairs (mutable)
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ParameterKey, &mut Value)> {
        self.values.iter_mut()
    }

    /// Iterate over all keys
    pub fn keys(&self) -> impl Iterator<Item = &ParameterKey> {
        self.values.keys()
    }

    /// Iterate over all values
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.values.values()
    }

    /// Iterate over all values (mutable)
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        self.values.values_mut()
    }

    // =========================================================================
    // Transformation
    // =========================================================================

    /// Map values to a new type
    pub fn map<F, T>(&self, mut f: F) -> HashMap<ParameterKey, T>
    where
        F: FnMut(&ParameterKey, &Value) -> T,
    {
        self.values
            .iter()
            .map(|(k, v)| (k.clone(), f(k, v)))
            .collect()
    }

    /// Filter and map values
    pub fn filter_map<F, T>(&self, mut f: F) -> HashMap<ParameterKey, T>
    where
        F: FnMut(&ParameterKey, &Value) -> Option<T>,
    {
        self.values
            .iter()
            .filter_map(|(k, v)| f(k, v).map(|t| (k.clone(), t)))
            .collect()
    }

    /// Merge another ParameterValues into this one
    ///
    /// Values from `other` will overwrite existing values with the same key.
    pub fn merge(&mut self, other: ParameterValues) {
        self.values.extend(other.values);
    }

    /// Merge with a custom merge function
    ///
    /// The merge function is called for each key that exists in both collections.
    pub fn merge_with<F>(&mut self, other: ParameterValues, mut f: F)
    where
        F: FnMut(&ParameterKey, Value, Value) -> Value,
    {
        for (key, other_value) in other.values {
            if let Some(existing) = self.values.remove(&key) {
                let merged = f(&key, existing, other_value);
                self.values.insert(key, merged);
            } else {
                self.values.insert(key, other_value);
            }
        }
    }

    // =========================================================================
    // Snapshot & Restore (for undo/redo)
    // =========================================================================

    /// Create a snapshot of current values
    ///
    /// Snapshots can be used for undo/redo functionality.
    #[must_use]
    pub fn snapshot(&self) -> ParameterSnapshot {
        ParameterSnapshot {
            values: self.values.clone(),
        }
    }

    /// Restore values from a snapshot
    ///
    /// This completely replaces the current values.
    pub fn restore(&mut self, snapshot: &ParameterSnapshot) {
        self.values = snapshot.values.clone();
    }

    /// Restore values from a snapshot, keeping unmodified values
    ///
    /// Only values present in the snapshot are restored.
    /// Other values remain unchanged.
    pub fn restore_partial(&mut self, snapshot: &ParameterSnapshot) {
        for (key, value) in &snapshot.values {
            self.values.insert(key.clone(), value.clone());
        }
    }

    // =========================================================================
    // Comparison & Diff
    // =========================================================================

    /// Get keys that differ from another ParameterValues
    #[must_use]
    pub fn diff_keys(&self, other: &ParameterValues) -> Vec<ParameterKey> {
        let mut diff = Vec::new();

        // Check for changed or removed values
        for (key, value) in &self.values {
            if other.get(key.clone()) != Some(value) {
                diff.push(key.clone());
            }
        }

        // Check for added values
        for key in other.keys() {
            if !self.contains(key.clone()) {
                diff.push(key.clone());
            }
        }

        diff
    }

    /// Create a diff showing what changed between two snapshots
    #[must_use]
    pub fn diff(&self, other: &ParameterValues) -> ParameterDiff {
        let mut added = HashMap::new();
        let mut removed = HashMap::new();
        let mut changed = HashMap::new();

        // Find removed and changed
        for (key, value) in &self.values {
            match other.get(key.clone()) {
                None => {
                    removed.insert(key.clone(), value.clone());
                }
                Some(other_value) if other_value != value => {
                    changed.insert(key.clone(), (value.clone(), other_value.clone()));
                }
                _ => {}
            }
        }

        // Find added
        for (key, value) in &other.values {
            if !self.contains(key.clone()) {
                added.insert(key.clone(), value.clone());
            }
        }

        ParameterDiff {
            added,
            removed,
            changed,
        }
    }

    // =========================================================================
    // Validation Helpers
    // =========================================================================

    /// Check if all required keys are present
    pub fn has_all_required(
        &self,
        required_keys: &[ParameterKey],
    ) -> Result<(), Vec<ParameterKey>> {
        let missing: Vec<_> = required_keys
            .iter()
            .filter(|k| !self.contains((*k).clone()))
            .cloned()
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    /// Check if any keys are present
    #[must_use]
    pub fn has_any(&self, keys: &[ParameterKey]) -> bool {
        keys.iter().any(|k| self.contains(k.clone()))
    }

    // =========================================================================
    // Internal Access
    // =========================================================================

    /// Get direct access to internal HashMap (read-only)
    #[must_use]
    pub fn as_map(&self) -> &HashMap<ParameterKey, Value> {
        &self.values
    }

    /// Get direct mutable access to internal HashMap
    ///
    /// # Safety
    ///
    /// This bypasses all validation. Use with caution.
    pub fn as_map_mut(&mut self) -> &mut HashMap<ParameterKey, Value> {
        &mut self.values
    }

    /// Consume and return the internal HashMap
    #[must_use]
    pub fn into_inner(self) -> HashMap<ParameterKey, Value> {
        self.values
    }
}

// =============================================================================
// Standard Trait Implementations
// =============================================================================

impl FromIterator<(ParameterKey, Value)> for ParameterValues {
    fn from_iter<T: IntoIterator<Item = (ParameterKey, Value)>>(iter: T) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

impl Extend<(ParameterKey, Value)> for ParameterValues {
    fn extend<T: IntoIterator<Item = (ParameterKey, Value)>>(&mut self, iter: T) {
        self.values.extend(iter);
    }
}

impl IntoIterator for ParameterValues {
    type Item = (ParameterKey, Value);
    type IntoIter = std::collections::hash_map::IntoIter<ParameterKey, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'a> IntoIterator for &'a ParameterValues {
    type Item = (&'a ParameterKey, &'a Value);
    type IntoIter = std::collections::hash_map::Iter<'a, ParameterKey, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl<'a> IntoIterator for &'a mut ParameterValues {
    type Item = (&'a ParameterKey, &'a mut Value);
    type IntoIter = std::collections::hash_map::IterMut<'a, ParameterKey, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

impl Index<&ParameterKey> for ParameterValues {
    type Output = Value;

    fn index(&self, key: &ParameterKey) -> &Self::Output {
        &self.values[key]
    }
}

impl IndexMut<&ParameterKey> for ParameterValues {
    fn index_mut(&mut self, key: &ParameterKey) -> &mut Self::Output {
        self.values.get_mut(key).expect("key not found")
    }
}

impl From<HashMap<ParameterKey, Value>> for ParameterValues {
    fn from(values: HashMap<ParameterKey, Value>) -> Self {
        Self { values }
    }
}

impl From<ParameterValues> for HashMap<ParameterKey, Value> {
    fn from(params: ParameterValues) -> Self {
        params.values
    }
}

// =============================================================================
// Snapshot
// =============================================================================

/// Snapshot of parameter values for undo/redo
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSnapshot {
    values: HashMap<ParameterKey, Value>,
}

impl ParameterSnapshot {
    /// Create an empty snapshot
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Get the number of values in the snapshot
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Check if the snapshot is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get a value from the snapshot
    #[must_use]
    pub fn get(&self, key: &ParameterKey) -> Option<&Value> {
        self.values.get(key)
    }

    /// Check if snapshot contains a key
    #[must_use]
    pub fn contains(&self, key: &ParameterKey) -> bool {
        self.values.contains_key(key)
    }
}

impl Default for ParameterSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Diff
// =============================================================================

/// Represents the difference between two ParameterValues
#[derive(Debug, Clone)]
pub struct ParameterDiff {
    /// Values that were added
    pub added: HashMap<ParameterKey, Value>,

    /// Values that were removed
    pub removed: HashMap<ParameterKey, Value>,

    /// Values that changed (old_value, new_value)
    pub changed: HashMap<ParameterKey, (Value, Value)>,
}

impl ParameterDiff {
    /// Check if there are any changes
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// Get total number of changes
    #[must_use]
    pub fn total_changes(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }

    /// Apply this diff to ParameterValues
    pub fn apply(&self, values: &mut ParameterValues) {
        // Remove deleted values
        for key in self.removed.keys() {
            values.remove(key.clone());
        }

        // Add new values
        for (key, value) in &self.added {
            values.set(key.clone(), value.clone());
        }

        // Update changed values
        for (key, (_old, new)) in &self.changed {
            values.set(key.clone(), new.clone());
        }
    }

    /// Reverse this diff (for undo)
    #[must_use]
    pub fn reverse(&self) -> Self {
        Self {
            added: self.removed.clone(),
            removed: self.added.clone(),
            changed: self
                .changed
                .iter()
                .map(|(k, (old, new))| (k.clone(), (new.clone(), old.clone())))
                .collect(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ParameterKey {
        ParameterKey::new(s).unwrap()
    }

    #[test]
    fn test_basic_operations() {
        let mut values = ParameterValues::new();

        values.set(key("name"), Value::text("Alice"));
        values.set(key("age"), Value::integer(30));

        assert_eq!(values.len(), 2);
        assert!(values.contains(key("name")));
        assert_eq!(values.get(key("name")), Some(&Value::text("Alice")));
    }

    #[test]
    fn test_typed_access() {
        let mut values = ParameterValues::new();
        values.set(key("count"), Value::integer(42));

        let count: i32 = values.get_typed(key("count")).unwrap();
        assert_eq!(count, 42);
    }

    #[test]
    fn test_iterator() {
        let values: ParameterValues =
            vec![(key("a"), Value::integer(1)), (key("b"), Value::integer(2))]
                .into_iter()
                .collect();

        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_snapshot() {
        let mut values = ParameterValues::new();
        values.set(key("x"), Value::integer(1));

        let snapshot = values.snapshot();

        values.set(key("x"), Value::integer(2));
        assert_eq!(values.get_typed::<i32>(key("x")).unwrap(), 2);

        values.restore(&snapshot);
        assert_eq!(values.get_typed::<i32>(key("x")).unwrap(), 1);
    }

    #[test]
    fn test_diff() {
        let mut v1 = ParameterValues::new();
        v1.set(key("a"), Value::integer(1));
        v1.set(key("b"), Value::integer(2));

        let mut v2 = ParameterValues::new();
        v2.set(key("a"), Value::integer(10));
        v2.set(key("c"), Value::integer(3));

        let diff = v1.diff(&v2);

        assert_eq!(diff.removed.len(), 1); // b removed
        assert_eq!(diff.added.len(), 1); // c added
        assert_eq!(diff.changed.len(), 1); // a changed
    }
}
