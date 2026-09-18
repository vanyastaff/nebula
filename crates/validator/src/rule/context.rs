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
        // A container root counts its keys and indices recursively. A scalar
        // root has no addressable children but the root itself is addressable
        // through `FieldPath::root()`, so it counts as one binding. That keeps
        // `len() == 0` exactly equivalent to `is_empty()`.
        let binding_count = if matches!(
            root,
            serde_json::Value::Object(_) | serde_json::Value::Array(_)
        ) {
            count_descendant_bindings(&root)
        } else {
            1
        };
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

    /// Number of addressable bindings in this context.
    ///
    /// For a `from_json` root this counts every reachable object key and array
    /// index, plus one for a scalar root (addressable as [`FieldPath::root`]).
    /// For `from_fields` it counts the supplied pairs.
    ///
    /// Contract: `len() == 0` if and only if `is_empty()`.
    pub const fn len(&self) -> usize {
        self.binding_count
    }

    /// True if this context binds nothing addressable.
    ///
    /// Contract: equivalent to `len() == 0`.
    pub fn is_empty(&self) -> bool {
        self.binding_count == 0
    }
}

/// Count every key and index reachable from `value`, iteratively.
///
/// A predicate context may own an arbitrarily deep JSON tree, so the walk uses
/// an explicit stack: the recursive form overflowed the process stack on a
/// deeply nested root. The caller ([`PredicateContext::from_json`]) runs this
/// before any traversal, so an over-deep root still costs one bounded walk.
fn count_descendant_bindings(value: &serde_json::Value) -> usize {
    let mut count = 0usize;
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            serde_json::Value::Object(object) => {
                count = count.saturating_add(object.len());
                pending.extend(object.values());
            },
            serde_json::Value::Array(array) => {
                count = count.saturating_add(array.len());
                pending.extend(array.iter());
            },
            _ => {},
        }
    }
    count
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
        assert_eq!(ctx.len(), 0);
    }

    #[test]
    fn len_and_is_empty_agree_for_every_root_shape() {
        // Containers with content bind something.
        let object = PredicateContext::from_json(json!({"a": 1}));
        assert_eq!(object.len(), 1);
        assert!(!object.is_empty());

        let array = PredicateContext::from_json(json!([1, 2]));
        assert_eq!(array.len(), 2);
        assert!(!array.is_empty());

        // Empty containers bind nothing at all.
        let empty_object = PredicateContext::from_json(json!({}));
        assert_eq!(empty_object.len(), 0);
        assert!(empty_object.is_empty());

        let empty_array = PredicateContext::from_json(json!([]));
        assert_eq!(empty_array.len(), 0);
        assert!(empty_array.is_empty());

        // A scalar root is addressable through FieldPath::root(), so it binds.
        let scalar = PredicateContext::from_json(json!("hello"));
        assert_eq!(scalar.len(), 1);
        assert!(!scalar.is_empty());
        assert_eq!(scalar.get(&FieldPath::root()), Some(&json!("hello")));

        let root_nulls = PredicateContext::from_json(json!(null));
        assert_eq!(root_nulls.len(), 1);
        assert!(!root_nulls.is_empty());

        // from_fields binds exactly the supplied pairs.
        let fields = PredicateContext::from_fields([(FieldPath::single("name"), json!("alice"))]);
        assert_eq!(fields.len(), 1);
        assert!(!fields.is_empty());
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
