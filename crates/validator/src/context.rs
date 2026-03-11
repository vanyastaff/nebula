//! Field value provider trait for context-aware rule evaluation.

/// Trait for accessing field values during rule evaluation.
///
/// Implemented by runtime value containers (e.g. `FieldValues` in the
/// parameter crate) so that context-predicate rules like `Eq`, `Set`,
/// `IsTrue` etc. can read sibling field values.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::FieldValueProvider;
/// use std::collections::HashMap;
///
/// struct Values(HashMap<String, serde_json::Value>);
///
/// impl FieldValueProvider for Values {
///     fn get_field(&self, key: &str) -> Option<&serde_json::Value> {
///         self.0.get(key)
///     }
/// }
/// ```
pub trait FieldValueProvider {
    /// Returns the JSON value for the given field key, or `None` if absent.
    fn get_field(&self, key: &str) -> Option<&serde_json::Value>;
}

/// Blanket implementation for `HashMap<String, serde_json::Value>`.
impl FieldValueProvider for std::collections::HashMap<String, serde_json::Value> {
    fn get_field(&self, key: &str) -> Option<&serde_json::Value> {
        self.get(key)
    }
}

/// Blanket implementation for `serde_json::Map`.
impl FieldValueProvider for serde_json::Map<String, serde_json::Value> {
    fn get_field(&self, key: &str) -> Option<&serde_json::Value> {
        self.get(key)
    }
}
