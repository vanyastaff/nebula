//! `PredicateContext` — typed context for `Predicate::evaluate`.
//!
//! Owns one JSON root and resolves `FieldPath` pointers on demand. Callers build
//! it once per evaluation round without cloning every addressable subtree.

use std::collections::HashMap;

use crate::foundation::FieldPath;

/// Typed field context for predicate evaluation. Construct via
/// `PredicateContext::from_json` or `PredicateContext::from_fields`.
#[derive(Clone, Default)]
pub struct PredicateContext {
    values: ContextValues,
    binding_count: usize,
    pending_paths: Vec<FieldPath>,
}

#[derive(Clone, Default)]
enum ContextValues {
    #[default]
    Empty,
    Root(serde_json::Value),
    Fields(HashMap<FieldPath, serde_json::Value>),
}

impl std::fmt::Debug for PredicateContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Field values may be secret-shaped pre-resolve; never print them.
        f.debug_struct("PredicateContext")
            .field("field_count", &self.len())
            .field("pending_count", &self.pending_paths.len())
            .finish_non_exhaustive()
    }
}

impl PredicateContext {
    /// Empty context — predicates see no fields.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from an iterator of `(FieldPath, Value)` pairs.
    pub fn from_fields<I: IntoIterator<Item = (FieldPath, serde_json::Value)>>(iter: I) -> Self {
        let fields: HashMap<_, _> = iter.into_iter().collect();
        let binding_count = fields.len();
        Self {
            values: ContextValues::Fields(fields),
            binding_count,
            pending_paths: Vec::new(),
        }
    }

    /// Own one JSON value for direct RFC6901 pointer lookup.
    pub fn from_json(root: serde_json::Value) -> Self {
        let binding_count = count_descendant_bindings(&root);
        Self {
            values: ContextValues::Root(root),
            binding_count,
            pending_paths: Vec::new(),
        }
    }

    /// Adds roots whose expression results are not available in this context.
    ///
    /// Pending roots affect both ancestors (incomplete containers) and
    /// descendants (unknown nested values). Build a fresh context after resolution.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_validator::{PredicateContext, foundation::FieldPath};
    /// let ctx = PredicateContext::new()
    ///     .with_pending_paths([FieldPath::from_segments(["settings", "computed"])]);
    /// assert!(ctx.is_pending(&FieldPath::single("settings")));
    /// assert!(!ctx.is_pending(&FieldPath::from_segments(["settings", "literal"])));
    /// ```
    #[must_use = "pending paths must be attached to the evaluation context"]
    pub fn with_pending_paths(mut self, paths: impl IntoIterator<Item = FieldPath>) -> Self {
        self.pending_paths.extend(paths);
        self
    }

    /// Whether this path overlaps an unresolved root by complete path segments.
    #[must_use]
    pub fn is_pending(&self, path: &FieldPath) -> bool {
        self.pending_paths
            .iter()
            .any(|pending| path.starts_with(pending) || pending.starts_with(path))
    }

    /// Fetch a stored value by path. Returns `None` if the field is absent.
    /// Pending metadata does not alter stored values; predicate evaluation
    /// checks availability before interpreting the value or its absence.
    pub fn get(&self, path: &FieldPath) -> Option<&serde_json::Value> {
        match &self.values {
            ContextValues::Empty => None,
            ContextValues::Root(root) => root.pointer(path.as_str()),
            ContextValues::Fields(fields) => fields.get(path),
        }
    }

    /// Number of stored non-root bindings.
    pub const fn len(&self) -> usize {
        self.binding_count
    }

    /// True if no fields are bound.
    pub fn is_empty(&self) -> bool {
        match &self.values {
            ContextValues::Empty => true,
            ContextValues::Root(root) => match root {
                serde_json::Value::Object(object) => object.is_empty(),
                serde_json::Value::Array(array) => array.is_empty(),
                _ => false,
            },
            ContextValues::Fields(fields) => fields.is_empty(),
        }
    }
}

fn count_descendant_bindings(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(object) => {
            object.len()
                + object
                    .values()
                    .map(count_descendant_bindings)
                    .sum::<usize>()
        },
        serde_json::Value::Array(array) => {
            array.len() + array.iter().map(count_descendant_bindings).sum::<usize>()
        },
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn top_level_keys_indexed_by_pointer() {
        let ctx = PredicateContext::from_json(json!({"name": "alice", "age": 30}));
        let name = ctx.get(&FieldPath::parse("name").unwrap());
        assert_eq!(name, Some(&json!("alice")));
    }

    #[test]
    fn nested_keys_indexed_recursively() {
        let ctx = PredicateContext::from_json(json!({"user": {"email": "x@y.z"}}));
        let email = ctx.get(&FieldPath::parse("/user/email").unwrap());
        assert_eq!(email, Some(&json!("x@y.z")));
        assert_eq!(ctx.len(), 2);
    }

    #[test]
    fn root_and_array_indices_use_rfc6901_lookup() {
        let value = json!({"items": [{"name": "first"}]});
        let ctx = PredicateContext::from_json(value.clone());

        assert_eq!(ctx.get(&FieldPath::root()), Some(&value));
        assert_eq!(
            ctx.get(&FieldPath::from_pointer("/items/0/name").unwrap()),
            Some(&json!("first"))
        );
        assert_eq!(ctx.len(), 3);
    }

    #[test]
    fn missing_field_returns_none() {
        let ctx = PredicateContext::from_json(json!({}));
        assert!(ctx.get(&FieldPath::parse("absent").unwrap()).is_none());
    }

    #[test]
    fn empty_context_is_empty() {
        let ctx = PredicateContext::new();
        assert!(ctx.is_empty());
    }

    #[test]
    fn debug_does_not_leak_field_values() {
        let ctx = PredicateContext::from_json(json!({"api_key": "s3cr3t-value", "n": 1}));
        let dbg = format!("{ctx:?}");
        assert!(
            !dbg.contains("s3cr3t-value"),
            "Debug must not print field values: {dbg}"
        );
        assert!(dbg.contains("PredicateContext"));
    }

    #[test]
    fn keys_with_pointer_metacharacters_are_escaped() {
        // JSON keys containing `/` or `~` must be stored under escaped pointer paths.
        let ctx = PredicateContext::from_json(serde_json::json!({"a/b": 1, "c~d": 2}));
        // Lookup via FieldPath::from_segments which applies the same escaping.
        let slash_key = FieldPath::from_segments(["a/b"]);
        let tilde_key = FieldPath::from_segments(["c~d"]);
        assert_eq!(ctx.get(&slash_key), Some(&serde_json::json!(1)));
        assert_eq!(ctx.get(&tilde_key), Some(&serde_json::json!(2)));
    }
}
