use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::Index;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Reserved object key that marks expression-backed runtime values.
pub const EXPRESSION_KEY: &str = "$expr";

/// Typed runtime value model used on top of the JSON wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// Plain JSON literal.
    Literal(serde_json::Value),
    /// Expression-backed value encoded as `{ "$expr": "..." }`.
    Expression(String),
    /// Mode selection encoded as `{ "mode": "...", "value"?: ... }`.
    Mode {
        /// Selected variant key.
        mode: String,
        /// Optional payload for the selected variant.
        value: Option<serde_json::Value>,
    },
}

impl FieldValue {
    /// Parses a typed value from JSON runtime data.
    #[must_use]
    pub fn from_json(value: &serde_json::Value) -> Self {
        if let Some(object) = value.as_object() {
            if object.len() == 1
                && let Some(expression) = object
                    .get(EXPRESSION_KEY)
                    .and_then(serde_json::Value::as_str)
            {
                return Self::Expression(expression.to_owned());
            }

            if let Some(mode) = object.get("mode").and_then(serde_json::Value::as_str) {
                return Self::Mode {
                    mode: mode.to_owned(),
                    value: object.get("value").cloned(),
                };
            }
        }

        Self::Literal(value.clone())
    }

    /// Converts this typed value to the JSON wire representation.
    #[must_use]
    pub fn into_json(self) -> serde_json::Value {
        match self {
            Self::Literal(value) => value,
            Self::Expression(expression) => {
                serde_json::json!({ EXPRESSION_KEY: expression })
            }
            Self::Mode { mode, value } => {
                let mut object = serde_json::Map::new();
                object.insert("mode".to_owned(), serde_json::Value::String(mode));
                if let Some(value) = value {
                    object.insert("value".to_owned(), value);
                }
                serde_json::Value::Object(object)
            }
        }
    }
}

/// Borrowed view of a mode selection value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModeValueRef<'a> {
    /// Selected mode key.
    pub mode: &'a str,
    /// Optional variant payload.
    pub value: Option<&'a serde_json::Value>,
}

/// Trait for numeric types supported by [`FieldValues::get_number`].
pub trait Numeric:
    Copy + PartialOrd + Debug + Send + Sync + Serialize + DeserializeOwned + 'static
{
    /// Parse this type from a [`serde_json::Value`].
    fn from_json(value: &serde_json::Value) -> Option<Self>;
}

impl Numeric for f64 {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        value.as_f64()
    }
}

impl Numeric for i64 {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
    }
}

impl Numeric for u64 {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        value.as_u64()
    }
}

impl Numeric for u16 {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        value.as_u64().and_then(|v| u16::try_from(v).ok())
    }
}

/// A set of parameter values, keyed by parameter key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldValues {
    #[serde(flatten)]
    values: HashMap<String, serde_json::Value>,
}

impl FieldValues {
    /// Create an empty value set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a value by parameter key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.values.get(key)
    }

    /// Set a value for a parameter key.
    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.values.insert(key.into(), value);
    }

    /// Set a typed runtime value for a parameter key.
    pub fn set_typed(&mut self, key: impl Into<String>, value: FieldValue) {
        self.values.insert(key.into(), value.into_json());
    }

    /// Set an expression-backed value.
    pub fn set_expression(&mut self, key: impl Into<String>, expression: impl Into<String>) {
        self.set_typed(key, FieldValue::Expression(expression.into()));
    }

    /// Set a mode selection value.
    pub fn set_mode(
        &mut self,
        key: impl Into<String>,
        mode: impl Into<String>,
        value: Option<serde_json::Value>,
    ) {
        self.set_typed(
            key,
            FieldValue::Mode {
                mode: mode.into(),
                value,
            },
        );
    }

    /// Remove a value by key, returning it if it existed.
    pub fn remove(&mut self, key: &str) -> Option<serde_json::Value> {
        self.values.remove(key)
    }

    /// Check whether a value exists for the given key.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Iterate over all keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }

    /// The number of values stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Try to get a value as a string reference.
    #[must_use]
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.values.get(key)?.as_str()
    }

    /// Get a value classified into the typed runtime model.
    #[must_use]
    pub fn get_typed(&self, key: &str) -> Option<FieldValue> {
        self.values.get(key).map(FieldValue::from_json)
    }

    /// Get an expression body if the value is expression-backed.
    #[must_use]
    pub fn get_expression(&self, key: &str) -> Option<&str> {
        let object = self.values.get(key)?.as_object()?;
        if object.len() != 1 {
            return None;
        }
        object
            .get(EXPRESSION_KEY)
            .and_then(serde_json::Value::as_str)
    }

    /// Get mode selection details if the value is mode-based.
    #[must_use]
    pub fn get_mode(&self, key: &str) -> Option<ModeValueRef<'_>> {
        let object = self.values.get(key)?.as_object()?;
        let mode = object.get("mode")?.as_str()?;
        Some(ModeValueRef {
            mode,
            value: object.get("value"),
        })
    }

    /// Try to get a value as f64.
    #[must_use]
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get_number(key)
    }

    /// Try to get a value as bool.
    #[must_use]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.values.get(key)?.as_bool()
    }

    /// Try to get a value as i64.
    #[must_use]
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get_number(key)
    }

    /// Try to get a numeric value as a specific Rust numeric type.
    #[must_use]
    pub fn get_number<T: Numeric>(&self, key: &str) -> Option<T> {
        self.values.get(key).and_then(T::from_json)
    }

    /// Try to get a value as array slice.
    #[must_use]
    pub fn get_array(&self, key: &str) -> Option<&[serde_json::Value]> {
        self.values.get(key)?.as_array().map(Vec::as_slice)
    }

    /// Try to get a value as object.
    #[must_use]
    pub fn get_object(&self, key: &str) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.values.get(key)?.as_object()
    }

    /// Try to deserialize a value to a specific type.
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails.
    pub fn get_as<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Option<Result<T, serde_json::Error>> {
        self.values.get(key).map(T::deserialize)
    }

    /// Set a value with automatic JSON conversion.
    pub fn set_json<T: serde::Serialize>(
        &mut self,
        key: impl Into<String>,
        value: T,
    ) -> Result<(), serde_json::Error> {
        let json_value = serde_json::to_value(value)?;
        self.values.insert(key.into(), json_value);
        Ok(())
    }

    /// Merge another value set into this one, overwriting existing keys.
    pub fn merge(&mut self, other: &Self) {
        for (k, v) in &other.values {
            self.values.insert(k.clone(), v.clone());
        }
    }

    /// Create a snapshot of the current values for later restore.
    #[must_use]
    pub fn snapshot(&self) -> FieldValuesSnapshot {
        FieldValuesSnapshot {
            values: self.values.clone(),
        }
    }

    /// Restore values from a previously taken snapshot.
    pub fn restore(&mut self, snapshot: &FieldValuesSnapshot) {
        self.values = snapshot.values.clone();
    }

    /// Compute the difference between this value set and another.
    #[must_use]
    pub fn diff(&self, other: &Self) -> FieldValuesDiff {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();

        for key in other.values.keys() {
            if !self.values.contains_key(key) {
                added.push(key.clone());
            }
        }

        for key in self.values.keys() {
            if !other.values.contains_key(key) {
                removed.push(key.clone());
            } else if self.values[key] != other.values[key] {
                changed.push(key.clone());
            }
        }

        added.sort();
        removed.sort();
        changed.sort();

        FieldValuesDiff {
            added,
            removed,
            changed,
        }
    }
}

impl FromIterator<(String, serde_json::Value)> for FieldValues {
    fn from_iter<I: IntoIterator<Item = (String, serde_json::Value)>>(iter: I) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

impl Index<&str> for FieldValues {
    type Output = serde_json::Value;

    fn index(&self, key: &str) -> &Self::Output {
        &self.values[key]
    }
}

/// A frozen copy of parameter values for snapshot/restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldValuesSnapshot {
    values: HashMap<String, serde_json::Value>,
}

impl nebula_validator::context::FieldValueProvider for FieldValues {
    fn get_field(&self, key: &str) -> Option<&serde_json::Value> {
        self.get(key)
    }
}

/// Describes the differences between two parameter value sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldValuesDiff {
    /// Keys present in `other` but not in `self`.
    pub added: Vec<String>,
    /// Keys present in `self` but not in `other`.
    pub removed: Vec<String>,
    /// Keys present in both but with different values.
    pub changed: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_is_empty() {
        let vals = FieldValues::new();
        assert!(vals.is_empty());
        assert_eq!(vals.len(), 0);
    }

    #[test]
    fn set_and_get() {
        let mut vals = FieldValues::new();
        vals.set("host", json!("localhost"));
        vals.set("port", json!(8080));

        assert_eq!(vals.get("host"), Some(&json!("localhost")));
        assert_eq!(vals.get("port"), Some(&json!(8080)));
        assert_eq!(vals.get("missing"), None);
        assert_eq!(vals.len(), 2);
    }

    #[test]
    fn remove() {
        let mut vals = FieldValues::new();
        vals.set("key", json!("value"));

        let removed = vals.remove("key");
        assert_eq!(removed, Some(json!("value")));
        assert!(vals.is_empty());
        assert!(vals.remove("key").is_none());
    }

    #[test]
    fn contains() {
        let mut vals = FieldValues::new();
        vals.set("host", json!("localhost"));

        assert!(vals.contains("host"));
        assert!(!vals.contains("port"));
    }

    #[test]
    fn keys_iterator() {
        let mut vals = FieldValues::new();
        vals.set("a", json!(1));
        vals.set("b", json!(2));

        let mut keys: Vec<&str> = vals.keys().collect();
        keys.sort();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn convenience_getters() {
        let mut vals = FieldValues::new();
        vals.set("name", json!("Alice"));
        vals.set("age", json!(30));
        vals.set("active", json!(true));
        vals.set("data", json!([1, 2, 3]));

        assert_eq!(vals.get_string("name"), Some("Alice"));
        assert_eq!(vals.get_string("age"), None);

        assert_eq!(vals.get_f64("age"), Some(30.0));
        assert_eq!(vals.get_f64("name"), None);
        assert_eq!(vals.get_bool("active"), Some(true));
        assert_eq!(vals.get_bool("name"), None);
    }

    #[test]
    fn get_number_preserves_integer_types() {
        let mut vals = FieldValues::new();
        vals.set("port", serde_json::json!(8080));
        vals.set("ratio", serde_json::json!(0.5));

        assert_eq!(vals.get_number::<u16>("port"), Some(8080));
        assert_eq!(vals.get_number::<i64>("port"), Some(8080));
        assert_eq!(vals.get_number::<f64>("ratio"), Some(0.5));
        assert_eq!(vals.get_number::<u16>("ratio"), None);
    }

    #[test]
    fn snapshot_and_restore() {
        let mut vals = FieldValues::new();
        vals.set("x", json!(1));
        vals.set("y", json!(2));

        let snap = vals.snapshot();

        vals.set("x", json!(99));
        vals.remove("y");
        vals.set("z", json!(3));
        assert_eq!(vals.get("x"), Some(&json!(99)));

        vals.restore(&snap);
        assert_eq!(vals.get("x"), Some(&json!(1)));
        assert_eq!(vals.get("y"), Some(&json!(2)));
        assert!(!vals.contains("z"));
    }

    #[test]
    fn diff_detects_changes() {
        let mut a = FieldValues::new();
        a.set("x", json!(1));
        a.set("y", json!(2));
        a.set("z", json!(3));

        let mut b = FieldValues::new();
        b.set("x", json!(1)); // same
        b.set("y", json!(99)); // changed
        b.set("w", json!(4)); // added

        let diff = a.diff(&b);
        assert_eq!(diff.added, vec!["w"]);
        assert_eq!(diff.removed, vec!["z"]);
        assert_eq!(diff.changed, vec!["y"]);
    }

    #[test]
    fn diff_empty_sets() {
        let a = FieldValues::new();
        let b = FieldValues::new();
        let diff = a.diff(&b);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.changed.is_empty());
    }

    #[test]
    fn from_iterator() {
        let vals: FieldValues = vec![("a".to_owned(), json!(1)), ("b".to_owned(), json!(2))]
            .into_iter()
            .collect();

        assert_eq!(vals.len(), 2);
        assert_eq!(vals.get("a"), Some(&json!(1)));
    }

    #[test]
    fn index_access() {
        let mut vals = FieldValues::new();
        vals.set("key", json!("value"));
        assert_eq!(vals["key"], json!("value"));
    }

    #[test]
    #[should_panic]
    fn index_missing_key_panics() {
        let vals = FieldValues::new();
        let _ = &vals["missing"];
    }

    #[test]
    fn serde_round_trip() {
        let mut vals = FieldValues::new();
        vals.set("host", json!("localhost"));
        vals.set("port", json!(8080));

        let json_str = serde_json::to_string(&vals).unwrap();
        let deserialized: FieldValues = serde_json::from_str(&json_str).unwrap();
        assert_eq!(vals, deserialized);
    }

    #[test]
    fn serde_flat_structure() {
        let mut vals = FieldValues::new();
        vals.set("name", json!("test"));

        let json_str = serde_json::to_string(&vals).unwrap();
        // Should be flat, not nested under "values"
        assert!(json_str.contains("\"name\":\"test\""));
        assert!(!json_str.contains("\"values\""));
    }

    #[test]
    fn typed_value_expression_roundtrip() {
        let mut vals = FieldValues::new();
        vals.set_expression("timeout", "inputs.retries * 1000");

        assert_eq!(
            vals.get_expression("timeout"),
            Some("inputs.retries * 1000")
        );
        assert_eq!(
            vals.get_typed("timeout"),
            Some(FieldValue::Expression("inputs.retries * 1000".to_owned()))
        );
    }

    #[test]
    fn typed_value_mode_roundtrip() {
        let mut vals = FieldValues::new();
        vals.set_mode("auth", "bearer", Some(json!({ "token": "abc" })));

        let mode = vals.get_mode("auth").expect("mode value expected");
        assert_eq!(mode.mode, "bearer");
        assert_eq!(mode.value, Some(&json!({ "token": "abc" })));
        assert_eq!(
            vals.get_typed("auth"),
            Some(FieldValue::Mode {
                mode: "bearer".to_owned(),
                value: Some(json!({ "token": "abc" })),
            })
        );
    }

    #[test]
    fn typed_value_literal_classification() {
        let mut vals = FieldValues::new();
        vals.set("port", json!(8080));

        assert_eq!(
            vals.get_typed("port"),
            Some(FieldValue::Literal(json!(8080)))
        );
    }
}
