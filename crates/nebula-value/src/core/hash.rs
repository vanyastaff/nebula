//! Hash implementations for Value
//!
//! This module provides HashableValue wrapper that can be used as HashMap key.
//!
//! **Important**: Value cannot directly implement Hash because Float doesn't implement Eq
//! (due to NaN != NaN in IEEE 754). HashableValue treats all NaN values as equal.

use crate::core::value::Value;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// Wrapper for Value that can be used as HashMap key
///
/// **WARNING**: This wrapper treats all NaN values as equal for hashing purposes.
/// This violates IEEE 754 semantics but is necessary for HashMap usage.
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use nebula_value::core::hash::HashableValue;
/// use nebula_value::Value;
///
/// let mut map = HashMap::new();
/// map.insert(HashableValue(Value::integer(42)), "answer");
/// map.insert(HashableValue(Value::text("key")), "value");
///
/// assert_eq!(map.get(&HashableValue(Value::integer(42))), Some(&"answer"));
/// ```
#[derive(Debug, Clone)]
pub struct HashableValue(pub Value);

impl HashableValue {
    /// Create a new hashable value
    pub fn new(value: Value) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub fn into_inner(self) -> Value {
        self.0
    }

    /// Get a reference to the inner value
    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

impl Hash for HashableValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash the type discriminant first
        self.0.kind().hash(state);

        match &self.0 {
            Value::Null => {
                // Nothing to hash beyond the discriminant
            }

            Value::Boolean(b) => {
                b.hash(state);
            }

            Value::Integer(i) => {
                i.value().hash(state);
            }

            Value::Float(f) => {
                if f.is_nan() {
                    // Normalize ALL NaN values to same hash
                    f64::to_bits(f64::NAN).hash(state);
                } else if f.value() == 0.0 {
                    // Normalize -0.0 and +0.0 to same hash
                    0.0f64.to_bits().hash(state);
                } else {
                    f.to_bits().hash(state);
                }
            }

            Value::Decimal(d) => {
                // Hash the string representation to ensure precision
                d.to_string().hash(state);
            }

            Value::Text(t) => {
                t.as_str().hash(state);
            }

            Value::Bytes(b) => {
                b.as_slice().hash(state);
            }

            Value::Array(arr) => {
                // Hash length and all elements
                arr.len().hash(state);
                for item in arr.iter() {
                    // Hash serde_json::Value (stored internally)
                    format!("{:?}", item).hash(state);
                }
            }

            Value::Object(obj) => {
                // Hash length and all key-value pairs (order-independent)
                obj.len().hash(state);

                // Collect keys, sort them for deterministic hashing
                let mut keys: Vec<_> = obj.keys().collect();
                keys.sort();

                for key in keys {
                    key.hash(state);
                    if let Some(value) = obj.get(key) {
                        // Hash serde_json::Value (stored internally)
                        format!("{:?}", value).hash(state);
                    }
                }
            }

            #[cfg(feature = "temporal")]
            Value::Date(d) => {
                d.hash(state);
            }

            #[cfg(feature = "temporal")]
            Value::Time(t) => {
                t.hash(state);
            }

            #[cfg(feature = "temporal")]
            Value::DateTime(dt) => {
                dt.hash(state);
            }

            #[cfg(feature = "temporal")]
            Value::Duration(dur) => {
                dur.hash(state);
            }
        }
    }
}

impl PartialEq for HashableValue {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            // Null
            (Value::Null, Value::Null) => true,

            // Boolean
            (Value::Boolean(a), Value::Boolean(b)) => a == b,

            // Integer
            (Value::Integer(a), Value::Integer(b)) => a == b,

            // Float - special handling for NaN
            (Value::Float(a), Value::Float(b)) => {
                if a.is_nan() && b.is_nan() {
                    // All NaN values are equal for HashMap purposes
                    true
                } else if a.value() == 0.0 && b.value() == 0.0 {
                    // -0.0 and +0.0 are equal
                    true
                } else {
                    a.total_cmp(b) == Ordering::Equal
                }
            }

            // Decimal
            (Value::Decimal(a), Value::Decimal(b)) => a == b,

            // Text
            (Value::Text(a), Value::Text(b)) => a == b,

            // Bytes
            (Value::Bytes(a), Value::Bytes(b)) => a == b,

            // Array
            (Value::Array(a), Value::Array(b)) => a == b,

            // Object
            (Value::Object(a), Value::Object(b)) => a == b,

            // Date
            #[cfg(feature = "temporal")]
            (Value::Date(a), Value::Date(b)) => a == b,

            // Time
            #[cfg(feature = "temporal")]
            (Value::Time(a), Value::Time(b)) => a == b,

            // DateTime
            #[cfg(feature = "temporal")]
            (Value::DateTime(a), Value::DateTime(b)) => a == b,

            // Duration
            #[cfg(feature = "temporal")]
            (Value::Duration(a), Value::Duration(b)) => a == b,

            // Different types
            _ => false,
        }
    }
}

impl Eq for HashableValue {}

impl From<Value> for HashableValue {
    fn from(value: Value) -> Self {
        Self(value)
    }
}

impl From<HashableValue> for Value {
    fn from(hashable: HashableValue) -> Self {
        hashable.0
    }
}

/// Extension trait for convenient HashableValue creation
pub trait HashableValueExt {
    /// Convert to HashableValue
    fn hashable(self) -> HashableValue;
}

impl HashableValueExt for Value {
    fn hashable(self) -> HashableValue {
        HashableValue(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn test_hashable_value_in_hashmap() {
        let mut map = HashMap::new();

        map.insert(HashableValue(Value::integer(42)), "int");
        map.insert(HashableValue(Value::text("key")), "text");
        map.insert(HashableValue(Value::boolean(true)), "bool");

        assert_eq!(map.get(&HashableValue(Value::integer(42))), Some(&"int"));
        assert_eq!(map.get(&HashableValue(Value::text("key"))), Some(&"text"));
        assert_eq!(map.get(&HashableValue(Value::boolean(true))), Some(&"bool"));
    }

    #[test]
    fn test_hashable_value_in_hashset() {
        let mut set = HashSet::new();

        set.insert(HashableValue(Value::integer(1)));
        set.insert(HashableValue(Value::integer(2)));
        set.insert(HashableValue(Value::integer(3)));
        set.insert(HashableValue(Value::integer(1))); // Duplicate

        assert_eq!(set.len(), 3);
        assert!(set.contains(&HashableValue(Value::integer(1))));
        assert!(set.contains(&HashableValue(Value::integer(2))));
        assert!(set.contains(&HashableValue(Value::integer(3))));
    }

    #[test]
    fn test_nan_equality_in_hashmap() {
        let mut map = HashMap::new();

        let nan1 = HashableValue(Value::float(f64::NAN));
        let nan2 = HashableValue(Value::float(f64::NAN));

        map.insert(nan1.clone(), "first");
        // This should REPLACE the first entry (all NaNs are equal)
        map.insert(nan2.clone(), "second");

        assert_eq!(map.len(), 1);
        assert_eq!(
            map.get(&HashableValue(Value::float(f64::NAN))),
            Some(&"second")
        );
    }

    #[test]
    fn test_zero_normalization() {
        let mut map = HashMap::new();

        map.insert(HashableValue(Value::float(0.0)), "positive");
        map.insert(HashableValue(Value::float(-0.0)), "negative");

        // -0.0 and +0.0 should hash to same value
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_hashable_equality() {
        let a = HashableValue(Value::integer(42));
        let b = HashableValue(Value::integer(42));
        let c = HashableValue(Value::integer(43));

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_hashable_float_equality() {
        let a = HashableValue(Value::float(3.14));
        let b = HashableValue(Value::float(3.14));
        let c = HashableValue(Value::float(2.71));

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_hashable_text_equality() {
        let a = HashableValue(Value::text("hello"));
        let b = HashableValue(Value::text("hello"));
        let c = HashableValue(Value::text("world"));

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_hashable_different_types() {
        let int = HashableValue(Value::integer(42));
        let float = HashableValue(Value::float(42.0));
        let text = HashableValue(Value::text("42"));

        assert_ne!(int, float);
        assert_ne!(int, text);
        assert_ne!(float, text);
    }

    #[test]
    fn test_hashable_value_ext() {
        let value = Value::integer(42).hashable();
        assert_eq!(value.as_value().as_integer().map(|i| i.value()), Some(42));
    }

    #[test]
    fn test_hashable_into_inner() {
        let hashable = HashableValue(Value::integer(42));
        let value = hashable.into_inner();
        assert_eq!(value.as_integer().map(|i| i.value()), Some(42));
    }

    #[test]
    fn test_hashable_from_value() {
        let value = Value::integer(42);
        let hashable = HashableValue::from(value);
        assert_eq!(
            hashable.as_value().as_integer().map(|i| i.value()),
            Some(42)
        );
    }

    #[test]
    fn test_hashable_to_value() {
        let hashable = HashableValue(Value::integer(42));
        let value = Value::from(hashable);
        assert_eq!(value.as_integer().map(|i| i.value()), Some(42));
    }

    #[test]
    fn test_hash_consistency() {
        use std::collections::hash_map::DefaultHasher;

        let value = HashableValue(Value::integer(42));

        let mut hasher1 = DefaultHasher::new();
        value.hash(&mut hasher1);
        let hash1 = hasher1.finish();

        let mut hasher2 = DefaultHasher::new();
        value.hash(&mut hasher2);
        let hash2 = hasher2.finish();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_array_hashing() {
        use crate::collections::array::ArrayBuilder;

        let arr1 = ArrayBuilder::new()
            .push(serde_json::json!(1))
            .push(serde_json::json!(2))
            .build()
            .unwrap();

        let arr2 = ArrayBuilder::new()
            .push(serde_json::json!(1))
            .push(serde_json::json!(2))
            .build()
            .unwrap();

        let h1 = HashableValue(Value::Array(arr1));
        let h2 = HashableValue(Value::Array(arr2));

        assert_eq!(h1, h2);
    }

    #[test]
    fn test_object_hashing() {
        use crate::collections::object::ObjectBuilder;

        let obj1 = ObjectBuilder::new()
            .insert("a", serde_json::json!(1))
            .insert("b", serde_json::json!(2))
            .build()
            .unwrap();

        let obj2 = ObjectBuilder::new()
            .insert("a", serde_json::json!(1))
            .insert("b", serde_json::json!(2))
            .build()
            .unwrap();

        let h1 = HashableValue(Value::Object(obj1));
        let h2 = HashableValue(Value::Object(obj2));

        assert_eq!(h1, h2);
    }
}
