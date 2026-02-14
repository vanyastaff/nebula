use std::collections::HashMap;
use std::ops::Index;

use serde::{Deserialize, Serialize};

/// A set of parameter values, keyed by parameter key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterValues {
    #[serde(flatten)]
    values: HashMap<String, serde_json::Value>,
}

impl ParameterValues {
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

    /// Try to get a value as f64.
    #[must_use]
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.values.get(key)?.as_f64()
    }

    /// Try to get a value as bool.
    #[must_use]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.values.get(key)?.as_bool()
    }

    /// Create a snapshot of the current values for later restore.
    #[must_use]
    pub fn snapshot(&self) -> ParameterSnapshot {
        ParameterSnapshot {
            values: self.values.clone(),
        }
    }

    /// Restore values from a previously taken snapshot.
    pub fn restore(&mut self, snapshot: &ParameterSnapshot) {
        self.values = snapshot.values.clone();
    }

    /// Compute the difference between this value set and another.
    #[must_use]
    pub fn diff(&self, other: &Self) -> ParameterDiff {
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

        ParameterDiff {
            added,
            removed,
            changed,
        }
    }
}

impl FromIterator<(String, serde_json::Value)> for ParameterValues {
    fn from_iter<I: IntoIterator<Item = (String, serde_json::Value)>>(iter: I) -> Self {
        Self {
            values: iter.into_iter().collect(),
        }
    }
}

impl Index<&str> for ParameterValues {
    type Output = serde_json::Value;

    fn index(&self, key: &str) -> &Self::Output {
        &self.values[key]
    }
}

/// A frozen copy of parameter values for snapshot/restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterSnapshot {
    values: HashMap<String, serde_json::Value>,
}

/// Describes the differences between two parameter value sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDiff {
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
        let vals = ParameterValues::new();
        assert!(vals.is_empty());
        assert_eq!(vals.len(), 0);
    }

    #[test]
    fn set_and_get() {
        let mut vals = ParameterValues::new();
        vals.set("host", json!("localhost"));
        vals.set("port", json!(8080));

        assert_eq!(vals.get("host"), Some(&json!("localhost")));
        assert_eq!(vals.get("port"), Some(&json!(8080)));
        assert_eq!(vals.get("missing"), None);
        assert_eq!(vals.len(), 2);
    }

    #[test]
    fn remove() {
        let mut vals = ParameterValues::new();
        vals.set("key", json!("value"));

        let removed = vals.remove("key");
        assert_eq!(removed, Some(json!("value")));
        assert!(vals.is_empty());
        assert!(vals.remove("key").is_none());
    }

    #[test]
    fn contains() {
        let mut vals = ParameterValues::new();
        vals.set("host", json!("localhost"));

        assert!(vals.contains("host"));
        assert!(!vals.contains("port"));
    }

    #[test]
    fn keys_iterator() {
        let mut vals = ParameterValues::new();
        vals.set("a", json!(1));
        vals.set("b", json!(2));

        let mut keys: Vec<&str> = vals.keys().collect();
        keys.sort();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn convenience_getters() {
        let mut vals = ParameterValues::new();
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
    fn snapshot_and_restore() {
        let mut vals = ParameterValues::new();
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
        let mut a = ParameterValues::new();
        a.set("x", json!(1));
        a.set("y", json!(2));
        a.set("z", json!(3));

        let mut b = ParameterValues::new();
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
        let a = ParameterValues::new();
        let b = ParameterValues::new();
        let diff = a.diff(&b);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.changed.is_empty());
    }

    #[test]
    fn from_iterator() {
        let vals: ParameterValues = vec![("a".to_owned(), json!(1)), ("b".to_owned(), json!(2))]
            .into_iter()
            .collect();

        assert_eq!(vals.len(), 2);
        assert_eq!(vals.get("a"), Some(&json!(1)));
    }

    #[test]
    fn index_access() {
        let mut vals = ParameterValues::new();
        vals.set("key", json!("value"));
        assert_eq!(vals["key"], json!("value"));
    }

    #[test]
    #[should_panic]
    fn index_missing_key_panics() {
        let vals = ParameterValues::new();
        let _ = &vals["missing"];
    }

    #[test]
    fn serde_round_trip() {
        let mut vals = ParameterValues::new();
        vals.set("host", json!("localhost"));
        vals.set("port", json!(8080));

        let json_str = serde_json::to_string(&vals).unwrap();
        let deserialized: ParameterValues = serde_json::from_str(&json_str).unwrap();
        assert_eq!(vals, deserialized);
    }

    #[test]
    fn serde_flat_structure() {
        let mut vals = ParameterValues::new();
        vals.set("name", json!("test"));

        let json_str = serde_json::to_string(&vals).unwrap();
        // Should be flat, not nested under "values"
        assert!(json_str.contains("\"name\":\"test\""));
        assert!(!json_str.contains("\"values\""));
    }
}
