//! Helper traits and utilities for working with Values.
//!
//! This module provides convenient extension traits and utilities
//! that make common operations more ergonomic.

use crate::collections::{Array, Object};
use crate::core::{Value, ValueResult};

/// Extension trait for Value with additional helper methods.
pub trait ValueExt {
    /// Check if this value is "truthy" in a boolean context.
    ///
    /// Truthy values:
    /// - `Boolean(true)`
    /// - Non-zero numbers
    /// - Non-empty strings
    /// - Non-empty collections
    /// - All temporal types
    ///
    /// Falsy values:
    /// - `Null`
    /// - `Boolean(false)`
    /// - Zero (integer, float, decimal)
    /// - Empty string
    /// - Empty collections
    /// - NaN (considered falsy)
    fn is_truthy(&self) -> bool;

    /// Check if this value is "falsy" in a boolean context.
    fn is_falsy(&self) -> bool {
        !self.is_truthy()
    }

    /// Get the value kind as a string.
    fn kind_name(&self) -> &'static str;

    /// Check if this value is a scalar (not a collection).
    fn is_scalar(&self) -> bool;

    /// Deep clone this value.
    ///
    /// This is the same as `clone()` but more explicit about
    /// the deep cloning behavior for nested structures.
    fn deep_clone(&self) -> Self;
}

impl ValueExt for Value {
    fn is_truthy(&self) -> bool {
        self.to_boolean()
    }

    fn kind_name(&self) -> &'static str {
        self.kind().name()
    }

    fn is_scalar(&self) -> bool {
        !self.is_collection()
    }

    fn deep_clone(&self) -> Self {
        self.clone()
    }
}

/// Extension trait for Array with additional helper methods.
pub trait ArrayExt {
    /// Get the first element.
    fn first(&self) -> Option<&Value>;

    /// Get the last element.
    fn last(&self) -> Option<&Value>;

    /// Check if the array contains any element matching the predicate.
    fn any<F>(&self, f: F) -> bool
    where
        F: FnMut(&Value) -> bool;

    /// Check if all elements match the predicate.
    fn all<F>(&self, f: F) -> bool
    where
        F: FnMut(&Value) -> bool;

    /// Map elements to a new array.
    fn map<F>(&self, f: F) -> Array
    where
        F: FnMut(&Value) -> Value;

    /// Filter elements by predicate.
    fn filter<F>(&self, f: F) -> Array
    where
        F: FnMut(&Value) -> bool;
}

impl ArrayExt for Array {
    fn first(&self) -> Option<&Value> {
        self.get(0)
    }

    fn last(&self) -> Option<&Value> {
        if self.is_empty() {
            None
        } else {
            self.get(self.len() - 1)
        }
    }

    fn any<F>(&self, mut f: F) -> bool
    where
        F: FnMut(&Value) -> bool,
    {
        self.iter().any(|v| f(v))
    }

    fn all<F>(&self, mut f: F) -> bool
    where
        F: FnMut(&Value) -> bool,
    {
        self.iter().all(|v| f(v))
    }

    fn map<F>(&self, mut f: F) -> Array
    where
        F: FnMut(&Value) -> Value,
    {
        let items: Vec<Value> = self.iter().map(|v| f(v)).collect();
        Array::from_vec(items)
    }

    fn filter<F>(&self, mut f: F) -> Array
    where
        F: FnMut(&Value) -> bool,
    {
        let items: Vec<Value> = self.iter().filter(|v| f(v)).cloned().collect();
        Array::from_vec(items)
    }
}

/// Extension trait for Object with additional helper methods.
pub trait ObjectExt {
    /// Get a value by path (dot notation).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::{Object, Value};
    /// use nebula_value::helpers::ObjectExt;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("user".to_string(), Value::from(serde_json::json!({
    ///         "name": "Alice",
    ///         "age": 30
    ///     }))),
    /// ]);
    ///
    /// // This would get obj["user"]["name"] if implemented
    /// // assert_eq!(obj.get_path("user.name"), Some(&Value::text("Alice")));
    /// ```
    fn has_key(&self, key: &str) -> bool;

    /// Get multiple keys at once.
    fn get_many<'a>(&'a self, keys: &[&str]) -> Vec<Option<&'a Value>>;

    /// Map values to a new object.
    fn map_values<F>(&self, f: F) -> Object
    where
        F: FnMut(&Value) -> Value;
}

impl ObjectExt for Object {
    fn has_key(&self, key: &str) -> bool {
        self.contains_key(key)
    }

    fn get_many<'a>(&'a self, keys: &[&str]) -> Vec<Option<&'a Value>> {
        keys.iter().map(|k| self.get(k)).collect()
    }

    fn map_values<F>(&self, mut f: F) -> Object
    where
        F: FnMut(&Value) -> Value,
    {
        let entries: Vec<(String, Value)> =
            self.entries().map(|(k, v)| (k.clone(), f(v))).collect();
        Object::from_iter(entries)
    }
}

/// Trait for types that can be converted to a Value with context.
pub trait IntoValueWithContext {
    /// Convert to Value, potentially using a context for validation or transformation.
    fn into_value_with_context(self, context: &str) -> ValueResult<Value>;
}

// Implement for common types
impl IntoValueWithContext for String {
    fn into_value_with_context(self, _context: &str) -> ValueResult<Value> {
        Ok(Value::text(self))
    }
}

impl IntoValueWithContext for i64 {
    fn into_value_with_context(self, _context: &str) -> ValueResult<Value> {
        Ok(Value::integer(self))
    }
}

impl IntoValueWithContext for bool {
    fn into_value_with_context(self, _context: &str) -> ValueResult<Value> {
        Ok(Value::boolean(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_ext_truthy() {
        assert!(Value::boolean(true).is_truthy());
        assert!(Value::integer(1).is_truthy());
        assert!(Value::text("hello").is_truthy());

        assert!(Value::null().is_falsy());
        assert!(Value::boolean(false).is_falsy());
        assert!(Value::integer(0).is_falsy());
        assert!(Value::text("").is_falsy());
    }

    #[test]
    fn test_value_ext_scalar() {
        assert!(Value::integer(42).is_scalar());
        assert!(Value::text("hello").is_scalar());
        assert!(!Value::array_empty().is_scalar());
        assert!(!Value::object_empty().is_scalar());
    }

    #[test]
    fn test_array_ext() {
        let arr = Array::from_vec(vec![
            Value::integer(1),
            Value::integer(2),
            Value::integer(3),
        ]);

        assert_eq!(arr.first(), Some(&Value::integer(1)));
        assert_eq!(arr.last(), Some(&Value::integer(3)));

        assert!(arr.any(|v| v.as_integer() == Some(crate::Integer::new(2))));
        assert!(arr.all(|v| v.is_integer()));

        let doubled = arr.map(|v| {
            if let Some(i) = v.as_integer() {
                Value::integer(i.value() * 2)
            } else {
                v.clone()
            }
        });
        assert_eq!(doubled.first(), Some(&Value::integer(2)));
    }

    #[test]
    fn test_object_ext() {
        let obj = Object::from_iter(vec![
            ("a".to_string(), Value::integer(1)),
            ("b".to_string(), Value::integer(2)),
        ]);

        assert!(obj.has_key("a"));
        assert!(!obj.has_key("c"));

        let values = obj.get_many(&["a", "b", "c"]);
        assert_eq!(values.len(), 3);
        assert!(values[0].is_some());
        assert!(values[1].is_some());
        assert!(values[2].is_none());
    }
}
