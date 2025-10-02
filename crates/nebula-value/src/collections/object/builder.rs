//! Builder pattern for Object construction
//!
//! Provides a fluent API for building objects with validation.

use crate::collections::Object;
use crate::core::limits::ValueLimits;
use crate::core::error::ValueResult;

// TEMP: using serde_json::Value as placeholder
type ValueItem = serde_json::Value;

/// Builder for creating Object with validation and limits
///
/// # Examples
///
/// ```
/// use nebula_value::collections::object::ObjectBuilder;
///
/// let object = ObjectBuilder::new()
///     .insert("name", serde_json::json!("Alice"))
///     .insert("age", serde_json::json!(30))
///     .build()
///     .unwrap();
///
/// assert_eq!(object.len(), 2);
/// ```
#[derive(Debug, Clone)]
pub struct ObjectBuilder {
    entries: Vec<(String, ValueItem)>,
    limits: Option<ValueLimits>,
}

impl ObjectBuilder {
    /// Create a new empty builder
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            limits: None,
        }
    }

    /// Create a builder with initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            limits: None,
        }
    }

    /// Set value limits for validation
    pub fn with_limits(mut self, limits: ValueLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Insert a key-value pair
    pub fn insert(mut self, key: impl Into<String>, value: ValueItem) -> Self {
        let key = key.into();
        self.entries.push((key, value));
        self
    }

    /// Insert a key-value pair with validation
    pub fn try_insert(mut self, key: impl Into<String>, value: ValueItem) -> ValueResult<Self> {
        let key = key.into();

        if let Some(ref limits) = self.limits {
            limits.check_string_bytes(key.len())?;
            limits.check_object_keys(self.entries.len() + 1)?;
        }

        self.entries.push((key, value));
        Ok(self)
    }

    /// Insert multiple key-value pairs
    pub fn extend<I>(mut self, entries: I) -> Self
    where
        I: IntoIterator<Item = (String, ValueItem)>,
    {
        self.entries.extend(entries);
        self
    }

    /// Insert multiple key-value pairs with validation
    pub fn try_extend<I>(mut self, entries: I) -> ValueResult<Self>
    where
        I: IntoIterator<Item = (String, ValueItem)>,
    {
        let entries: Vec<_> = entries.into_iter().collect();

        if let Some(ref limits) = self.limits {
            limits.check_object_keys(self.entries.len() + entries.len())?;

            for (key, _) in &entries {
                limits.check_string_bytes(key.len())?;
            }
        }

        self.entries.extend(entries);
        Ok(self)
    }

    /// Remove a key
    pub fn remove(mut self, key: &str) -> Self {
        self.entries.retain(|(k, _)| k != key);
        self
    }

    /// Clear all entries
    pub fn clear(mut self) -> Self {
        self.entries.clear();
        self
    }

    /// Get the current number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the builder is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Check if a key exists
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }

    /// Merge with another ObjectBuilder
    pub fn merge(mut self, other: ObjectBuilder) -> Self {
        self.entries.extend(other.entries);
        self
    }

    /// Build the final Object
    pub fn build(self) -> ValueResult<Object> {
        if let Some(ref limits) = self.limits {
            limits.check_object_keys(self.entries.len())?;

            for (key, _) in &self.entries {
                limits.check_string_bytes(key.len())?;
            }
        }

        Ok(Object::from_iter(self.entries))
    }

    /// Build without validation (unsafe)
    pub fn build_unchecked(self) -> Object {
        Object::from_iter(self.entries)
    }
}

impl Default for ObjectBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience macro for building objects
///
/// # Examples
///
/// ```ignore
/// use nebula_value::object;
///
/// let obj = object! {
///     "name" => "Alice",
///     "age" => 30,
/// };
/// ```
#[macro_export]
macro_rules! object {
    () => {
        $crate::collections::Object::new()
    };
    ($($key:expr => $value:expr),+ $(,)?) => {
        $crate::collections::object::ObjectBuilder::new()
            $(.insert($key, serde_json::json!($value)))+
            .build()
            .expect("Object construction failed")
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_empty() {
        let object = ObjectBuilder::new().build().unwrap();
        assert_eq!(object.len(), 0);
        assert!(object.is_empty());
    }

    #[test]
    fn test_builder_insert() {
        let object = ObjectBuilder::new()
            .insert("name", serde_json::json!("Alice"))
            .insert("age", serde_json::json!(30))
            .insert("active", serde_json::json!(true))
            .build()
            .unwrap();

        assert_eq!(object.len(), 3);
        assert_eq!(object.get("name"), Some(&serde_json::json!("Alice")));
        assert_eq!(object.get("age"), Some(&serde_json::json!(30)));
        assert_eq!(object.get("active"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn test_builder_with_capacity() {
        let builder = ObjectBuilder::with_capacity(10);
        assert_eq!(builder.len(), 0);
    }

    #[test]
    fn test_builder_extend() {
        let object = ObjectBuilder::new()
            .extend(vec![
                ("x".to_string(), serde_json::json!(1)),
                ("y".to_string(), serde_json::json!(2)),
            ])
            .build()
            .unwrap();

        assert_eq!(object.len(), 2);
    }

    #[test]
    fn test_builder_with_limits() {
        let limits = ValueLimits {
            max_object_keys: 2,
            ..Default::default()
        };

        let result = ObjectBuilder::new()
            .with_limits(limits)
            .insert("a", serde_json::json!(1))
            .insert("b", serde_json::json!(2))
            .insert("c", serde_json::json!(3))
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_try_insert_exceeds_limit() {
        let limits = ValueLimits {
            max_object_keys: 2,
            ..Default::default()
        };

        let result = ObjectBuilder::new()
            .with_limits(limits)
            .try_insert("a", serde_json::json!(1)).unwrap()
            .try_insert("b", serde_json::json!(2)).unwrap()
            .try_insert("c", serde_json::json!(3));

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_remove() {
        let object = ObjectBuilder::new()
            .insert("a", serde_json::json!(1))
            .insert("b", serde_json::json!(2))
            .insert("c", serde_json::json!(3))
            .remove("b")
            .build()
            .unwrap();

        assert_eq!(object.len(), 2);
        assert!(object.contains_key("a"));
        assert!(!object.contains_key("b"));
        assert!(object.contains_key("c"));
    }

    #[test]
    fn test_builder_clear() {
        let object = ObjectBuilder::new()
            .insert("a", serde_json::json!(1))
            .insert("b", serde_json::json!(2))
            .clear()
            .build()
            .unwrap();

        assert_eq!(object.len(), 0);
    }

    #[test]
    fn test_builder_merge() {
        let builder1 = ObjectBuilder::new()
            .insert("a", serde_json::json!(1));

        let builder2 = ObjectBuilder::new()
            .insert("b", serde_json::json!(2));

        let object = builder1.merge(builder2).build().unwrap();

        assert_eq!(object.len(), 2);
        assert!(object.contains_key("a"));
        assert!(object.contains_key("b"));
    }

    #[test]
    fn test_builder_contains_key() {
        let builder = ObjectBuilder::new()
            .insert("name", serde_json::json!("Alice"));

        assert!(builder.contains_key("name"));
        assert!(!builder.contains_key("age"));
    }

    #[test]
    fn test_builder_key_length_limit() {
        let limits = ValueLimits {
            max_string_bytes: 5,
            ..Default::default()
        };

        let result = ObjectBuilder::new()
            .with_limits(limits)
            .try_insert("very_long_key_name", serde_json::json!(1));

        assert!(result.is_err());
    }
}