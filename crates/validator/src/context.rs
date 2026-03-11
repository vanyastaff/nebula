//! Field value provider trait for context-aware rule evaluation.
//!
//! The [`FieldValueProvider`] trait is used by context-predicate rules
//! (e.g. [`Rule::Eq`](crate::Rule::Eq), [`Rule::Set`](crate::Rule::Set))
//! to read sibling field values during [`Rule::evaluate`](crate::Rule::evaluate).
//!
//! Blanket implementations are provided for [`HashMap<String, Value>`](std::collections::HashMap)
//! and [`serde_json::Map<String, Value>`].

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

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn hashmap_get_existing_field() {
        let mut map = HashMap::new();
        map.insert("name".to_owned(), json!("Alice"));
        assert_eq!(map.get_field("name"), Some(&json!("Alice")));
    }

    #[test]
    fn hashmap_get_missing_field() {
        let map: HashMap<String, serde_json::Value> = HashMap::new();
        assert_eq!(map.get_field("name"), None);
    }

    #[test]
    fn serde_map_get_existing_field() {
        let mut map = serde_json::Map::new();
        map.insert("age".into(), json!(30));
        assert_eq!(map.get_field("age"), Some(&json!(30)));
    }

    #[test]
    fn serde_map_get_missing_field() {
        let map = serde_json::Map::new();
        assert_eq!(map.get_field("age"), None);
    }

    #[test]
    fn provider_works_with_rule_evaluate() {
        use crate::rule::Rule;

        let mut map = HashMap::new();
        map.insert("status".to_owned(), json!("active"));

        let rule = Rule::Eq {
            field: "status".into(),
            value: json!("active"),
        };
        assert!(rule.evaluate(&map));
    }

    #[test]
    fn serde_map_works_with_rule_evaluate() {
        use crate::rule::Rule;

        let mut map = serde_json::Map::new();
        map.insert("enabled".into(), json!(true));

        let rule = Rule::IsTrue {
            field: "enabled".into(),
        };
        assert!(rule.evaluate(&map));
    }
}
