//! `PredicateContext` — typed context for `Predicate::evaluate`.
//!
//! Wraps a `FieldPath`-keyed map of values. Callers build it from JSON
//! once per evaluation round.

use std::collections::HashMap;

use crate::foundation::FieldPath;

/// Typed field context for predicate evaluation. Construct via
/// `PredicateContext::from_json` or `PredicateContext::from_fields`.
#[derive(Debug, Clone, Default)]
pub struct PredicateContext {
    fields: HashMap<FieldPath, serde_json::Value>,
}

impl PredicateContext {
    /// Empty context — predicates see no fields.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from an iterator of `(FieldPath, Value)` pairs.
    pub fn from_fields<I: IntoIterator<Item = (FieldPath, serde_json::Value)>>(iter: I) -> Self {
        Self {
            fields: iter.into_iter().collect(),
        }
    }

    /// Flatten a JSON object into a FieldPath-keyed map.
    ///
    /// Top-level keys map to `/key` pointers. Nested objects get recursive
    /// `/a/b` keys. Arrays are stored as-is under their parent path
    /// (callers can extend to array-index paths if needed).
    pub fn from_json(obj: &serde_json::Value) -> Self {
        let mut fields = HashMap::new();
        if let Some(m) = obj.as_object() {
            collect_paths(&mut fields, None, m);
        }
        Self { fields }
    }

    /// Fetch a value by path. Returns `None` if the field is absent.
    pub fn get(&self, path: &FieldPath) -> Option<&serde_json::Value> {
        self.fields.get(path)
    }

    /// Number of stored field bindings.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// True if no fields are bound.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

fn collect_paths(
    out: &mut HashMap<FieldPath, serde_json::Value>,
    prefix: Option<&FieldPath>,
    obj: &serde_json::Map<String, serde_json::Value>,
) {
    for (k, v) in obj {
        let path = match prefix {
            None => FieldPath::single(k),
            Some(p) => p.push(k),
        };
        out.insert(path.clone(), v.clone());
        if let Some(nested) = v.as_object() {
            collect_paths(out, Some(&path), nested);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn top_level_keys_indexed_by_pointer() {
        let ctx = PredicateContext::from_json(&json!({"name": "alice", "age": 30}));
        let name = ctx.get(&FieldPath::parse("name").unwrap());
        assert_eq!(name, Some(&json!("alice")));
    }

    #[test]
    fn nested_keys_indexed_recursively() {
        let ctx = PredicateContext::from_json(&json!({"user": {"email": "x@y.z"}}));
        let email = ctx.get(&FieldPath::parse("/user/email").unwrap());
        assert_eq!(email, Some(&json!("x@y.z")));
    }

    #[test]
    fn missing_field_returns_none() {
        let ctx = PredicateContext::from_json(&json!({}));
        assert!(ctx.get(&FieldPath::parse("absent").unwrap()).is_none());
    }

    #[test]
    fn empty_context_is_empty() {
        let ctx = PredicateContext::new();
        assert!(ctx.is_empty());
    }

    #[test]
    fn keys_with_pointer_metacharacters_are_escaped() {
        // JSON keys containing `/` or `~` must be stored under escaped pointer paths.
        let ctx = PredicateContext::from_json(&serde_json::json!({"a/b": 1, "c~d": 2}));
        // Lookup via FieldPath::from_segments which applies the same escaping.
        let slash_key = FieldPath::from_segments(["a/b"]).unwrap();
        let tilde_key = FieldPath::from_segments(["c~d"]).unwrap();
        assert_eq!(ctx.get(&slash_key), Some(&serde_json::json!(1)));
        assert_eq!(ctx.get(&tilde_key), Some(&serde_json::json!(2)));
    }
}
