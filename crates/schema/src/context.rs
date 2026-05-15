//! Builds the `nebula_validator::PredicateContext` for a schema's
//! visibility/required evaluation.
//!
//! Walks the schema field tree and the value tree together, emitting
//! RFC-6901 pointers (`/a/b`) for non-secret `Literal` values at any
//! depth, and excluding any `Field::Secret` subtree by schema type —
//! pre-resolve a secret is `FieldValue::Literal(plaintext)`, so a
//! runtime-tag check would leak it. Nested resolution is required: a
//! flat top-level-only context makes every nested predicate fail open.

use indexmap::IndexMap;
use nebula_validator::{PredicateContext, foundation::FieldPath};

use crate::{
    field::Field,
    key::FieldKey,
    value::{FieldValue, FieldValues},
};

/// Build a `PredicateContext` from the value tree, recursively, excluding
/// every field declared as `Field::Secret` at any depth.
///
/// Only `FieldValue::Literal` leaves are addressable by predicates;
/// expression / list / mode / secret-sentinel subtrees are non-addressable.
#[must_use]
pub fn predicate_context_for(fields: &[Field], values: &FieldValues) -> PredicateContext {
    let mut pairs: Vec<(FieldPath, serde_json::Value)> = Vec::new();
    collect_non_secret(fields, values.as_map(), None, &mut pairs);
    PredicateContext::from_fields(pairs)
}

/// Recurse fields <-> values in lockstep, scrubbing `Field::Secret` at every
/// level and descending `Field::Object` <-> `FieldValue::Object`.
fn collect_non_secret(
    fields: &[Field],
    values: &IndexMap<FieldKey, FieldValue>,
    prefix: Option<&FieldPath>,
    out: &mut Vec<(FieldPath, serde_json::Value)>,
) {
    for field in fields {
        if matches!(field, Field::Secret(_)) {
            continue; // exclude secret-typed fields by schema type, any depth
        }
        let key = field.key();
        let Some(val) = values.get(key) else { continue };
        let path = match prefix {
            None => FieldPath::single(key.as_str()),
            Some(p) => p.push(key.as_str()),
        };
        match (field, val) {
            (Field::Object(obj), FieldValue::Object(sub)) => {
                collect_non_secret(obj.fields.as_slice(), sub, Some(&path), out);
            },
            (_, FieldValue::Literal(v)) => {
                out.push((path, v.clone()));
            },
            // Expression / SecretLiteral / List / Mode subtrees are
            // non-addressable by predicates.
            _ => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn literal_field_is_visible_to_predicates() {
        let fields = vec![Field::from(Field::string(FieldKey::new("name").unwrap()))];
        let mut values = FieldValues::new();
        values.set(
            FieldKey::new("name").unwrap(),
            FieldValue::Literal(json!("alice")),
        );
        let ctx = predicate_context_for(&fields, &values);
        assert_eq!(
            ctx.get(&FieldPath::parse("name").unwrap()),
            Some(&json!("alice"))
        );
    }

    #[test]
    fn pre_resolve_plaintext_secret_is_scrubbed_by_schema_type() {
        // A Field::Secret holding a pre-resolve plaintext Literal MUST NOT
        // enter the predicate context. The old runtime-tag scrub failed this.
        let fields = vec![Field::from(Field::secret(
            FieldKey::new("api_key").unwrap(),
        ))];
        let mut values = FieldValues::new();
        values.set(
            FieldKey::new("api_key").unwrap(),
            FieldValue::Literal(json!("s3cr3t-plaintext")),
        );
        let ctx = predicate_context_for(&fields, &values);
        assert!(
            ctx.get(&FieldPath::parse("api_key").unwrap()).is_none(),
            "secret-typed field must be excluded from the predicate context"
        );
    }

    #[test]
    fn predicate_context_debug_redacts_keys_and_values() {
        // Even for NON-secret fields that are legitimately in the context,
        // Debug prints neither keys nor values (only a count). Pins the full
        // "no keys, no values" redaction guarantee.
        let fields = vec![Field::from(Field::string(FieldKey::new("region").unwrap()))];
        let mut values = FieldValues::new();
        values.set(
            FieldKey::new("region").unwrap(),
            FieldValue::Literal(json!("eu-secret-marker")),
        );
        let ctx = predicate_context_for(&fields, &values);
        let dbg = format!("{ctx:?}");
        assert!(
            dbg.contains("PredicateContext"),
            "must name the type: {dbg}"
        );
        assert!(
            !dbg.contains("eu-secret-marker"),
            "must not print values: {dbg}"
        );
        assert!(!dbg.contains("region"), "must not print keys: {dbg}");
    }
}
