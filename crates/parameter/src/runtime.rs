//! Runtime-facing types for v2 parameter handling.

pub use crate::error::ParameterError;
pub use crate::values::{ModeValueRef, ParameterValue, ParameterValues};

// Backward compat
#[deprecated(note = "renamed to ParameterValue")]
pub use crate::values::ParameterValue as FieldValue;
#[deprecated(note = "renamed to ParameterValues")]
pub use crate::values::ParameterValues as FieldValues;

/// Schema-bound validated values view.
///
/// Cannot be constructed outside the crate — only produced by
/// [`Schema::validate`](crate::collection::ParameterCollection::validate) or
/// [`Schema::validate_with_profile`](crate::collection::ParameterCollection::validate_with_profile).
#[derive(Debug, Clone)]
pub struct ValidatedValues {
    values: ParameterValues,
}

impl ValidatedValues {
    /// Creates a validated wrapper from runtime values.
    ///
    /// Not publicly constructible — use [`Schema::validate`](crate::collection::ParameterCollection::validate).
    pub(crate) fn new(values: ParameterValues) -> Self {
        Self { values }
    }

    /// Accesses the underlying runtime values.
    #[must_use]
    pub fn raw(&self) -> &ParameterValues {
        &self.values
    }

    /// Consumes the wrapper and returns the raw values.
    #[must_use]
    pub fn into_inner(self) -> ParameterValues {
        self.values
    }

    /// Returns the value for `key`, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.values.get(key)
    }

    /// Returns the value as a string, if it is one.
    #[must_use]
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.values.get_string(key)
    }

    /// Returns the value as a bool, if it is one.
    #[must_use]
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.values.get_bool(key)
    }

    /// Returns the value as f64, if it is numeric.
    #[must_use]
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.values.get_f64(key)
    }

    /// Returns the value as i64, if it is an integer.
    #[must_use]
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.values.get_i64(key)
    }

    /// Returns the value as an array slice, if it is one.
    #[must_use]
    pub fn get_array(&self, key: &str) -> Option<&[serde_json::Value]> {
        self.values.get_array(key)
    }

    /// Returns the value as a JSON object, if it is one.
    #[must_use]
    pub fn get_object(&self, key: &str) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.values.get_object(key)
    }

    /// Returns the mode selection details, if the value is mode-based.
    #[must_use]
    pub fn get_mode(&self, key: &str) -> Option<ModeValueRef<'_>> {
        self.values.get_mode(key)
    }

    /// Checks whether a value exists for `key`.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains(key)
    }

    /// Returns the number of values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns `true` if there are no values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl std::ops::Index<&str> for ValidatedValues {
    type Output = serde_json::Value;

    fn index(&self, key: &str) -> &Self::Output {
        &self.values[key]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_validated(pairs: &[(&str, serde_json::Value)]) -> ValidatedValues {
        let mut values = ParameterValues::new();
        for (k, v) in pairs {
            values.set(*k, v.clone());
        }
        ValidatedValues::new(values)
    }

    #[test]
    fn get_delegates_to_inner() {
        let v = make_validated(&[("host", json!("localhost"))]);
        assert_eq!(v.get("host"), Some(&json!("localhost")));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    fn get_string_delegates() {
        let v = make_validated(&[("name", json!("Alice"))]);
        assert_eq!(v.get_string("name"), Some("Alice"));
    }

    #[test]
    fn get_bool_delegates() {
        let v = make_validated(&[("active", json!(true))]);
        assert_eq!(v.get_bool("active"), Some(true));
    }

    #[test]
    fn get_f64_delegates() {
        let v = make_validated(&[("score", json!(42.5))]);
        assert_eq!(v.get_f64("score"), Some(42.5));
    }

    #[test]
    fn get_i64_delegates() {
        let v = make_validated(&[("port", json!(8080))]);
        assert_eq!(v.get_i64("port"), Some(8080));
    }

    #[test]
    fn contains_delegates() {
        let v = make_validated(&[("key", json!("val"))]);
        assert!(v.contains("key"));
        assert!(!v.contains("other"));
    }

    #[test]
    fn len_and_is_empty_delegate() {
        let empty = make_validated(&[]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let v = make_validated(&[("a", json!(1)), ("b", json!(2))]);
        assert_eq!(v.len(), 2);
        assert!(!v.is_empty());
    }

    #[test]
    fn index_delegates() {
        let v = make_validated(&[("key", json!("val"))]);
        assert_eq!(v["key"], json!("val"));
    }

    #[test]
    fn get_mode_delegates() {
        let mut values = ParameterValues::new();
        values.set_mode("auth", "bearer", Some(json!({"token": "abc"})));
        let v = ValidatedValues::new(values);
        let mode = v.get_mode("auth").expect("should have mode");
        assert_eq!(mode.mode, "bearer");
    }

    #[test]
    fn get_array_delegates() {
        let v = make_validated(&[("items", json!([1, 2, 3]))]);
        let arr = v.get_array("items").expect("should have array");
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn get_object_delegates() {
        let v = make_validated(&[("config", json!({"host": "localhost"}))]);
        let obj = v.get_object("config").expect("should have object");
        assert_eq!(obj.get("host"), Some(&json!("localhost")));
    }
}
