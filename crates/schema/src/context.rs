//! Builds the `nebula_validator::PredicateContext` for a schema's
//! visibility/required evaluation.
//!
//! Walks the schema field tree and the value tree together, emitting
//! RFC-6901 pointers (`/a/b`) for non-secret `Literal` values at any
//! depth, and excluding any `Field::Secret` subtree by schema type —
//! pre-resolve a secret is `FieldValue::Literal(plaintext)`, so a
//! runtime-tag check would leak it. Nested resolution is required: a
//! flat top-level-only context makes every nested predicate fail open.
//!
//! A structured-typed field (`Object`/`List`/`Mode`) whose value is not the
//! matching structured shape is non-addressable: scrub-by-type holds even
//! for unvalidated [`FieldValues::set`] input — a `Literal` blob handed to a
//! structured field never enters the context, so a secret nested inside such
//! a blob cannot leak. Non-secret leaves nested under `Field::List` /
//! `Field::Mode` are likewise unreachable to `When` predicates: a deliberate
//! capability boundary matching prior behaviour, not only a secret guard.

use indexmap::IndexMap;
use nebula_validator::{PredicateContext, foundation::FieldPath};

use crate::{
    field::Field,
    key::FieldKey,
    lint::{AddressableKind, walk_addressable_paths},
    path::FieldPath as SchemaPath,
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
/// level, descending `Field::Object` <-> `FieldValue::Object`, and treating
/// any structured-typed field (`Object`/`List`/`Mode`) whose value is not the
/// matching structured shape as non-addressable (no `Literal` blob escapes a
/// structured field).
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
            // A structured-typed field with any non-matching value shape is
            // non-addressable. This MUST precede the blanket `Literal` arm so
            // a `Literal` blob handed to an `Object`/`List`/`Mode` field (e.g.
            // via the public unvalidated `FieldValues::set`) is never pushed —
            // it could otherwise carry a nested secret's plaintext.
            (Field::Object(_) | Field::List(_) | Field::Mode(_), _) => {},
            // Reached only for scalar-typed leaves now.
            (_, FieldValue::Literal(v)) => {
                out.push((path, v.clone()));
            },
            // Expression / SecretLiteral / nested non-literal subtrees are
            // non-addressable by predicates.
            _ => {},
        }
    }
}

/// Build the predicate context for **root rules**: every non-secret
/// addressable leaf (objects, list-item objects, mode-variant payloads — the
/// *same* traversal as the secret-value-predicate lint, via the single-owner
/// [`walk_addressable_paths`]), excluding `Field::Secret` by schema type and
/// any container node that has a secret descendant.
///
/// Secret-safe by construction: the shared walker visits only addressable
/// *leaves*. `Field::Secret` leaves yield [`AddressableKind::Secret`] and are
/// never pushed; only non-secret scalar `FieldValue::Literal` leaves are
/// emitted (mirroring [`collect_non_secret`]'s leaf rule). A container
/// (`Object`/`List`/`Mode`) is never a leaf, so a container node — which would
/// serialize a secret descendant's plaintext into a `Contains`-reachable blob
/// — is structurally impossible to emit.
///
/// This closes the root-rule secret-plaintext exposure (root rules previously
/// ran against `PredicateContext::from_json` of the full unscrubbed submission)
/// **without** making legal non-secret nested root predicates fail open: a
/// nested-object non-secret value (e.g. `/policy/region`) still resolves, so a
/// legitimate root guard keyed on it continues to fire. (List-item / mode
/// payload leaves are keyed by anonymous/variant schema segments that no
/// concrete value-tree pointer resolves to — identical to the prior `from_json`
/// behaviour for those shapes, so no new fail-open is introduced; this is the
/// documented capability boundary, additive to the build-time secret lint.)
#[must_use]
pub fn root_predicate_context_for(fields: &[Field], values: &FieldValues) -> PredicateContext {
    let mut pairs: Vec<(FieldPath, serde_json::Value)> = Vec::new();
    walk_addressable_paths(fields, &mut |segs, kind| {
        // Never emit a secret leaf — its plaintext must not enter any context.
        if kind != AddressableKind::NonSecretLeaf {
            return;
        }
        // Resolve the value at this addressable schema path. Segments are
        // already valid `FieldKey` strings (the walker derives them from
        // `field.key()` / validated variant keys); a malformed one cannot name
        // a legal leaf, so skip it rather than coerce.
        let mut schema_path = SchemaPath::root();
        for seg in segs {
            let Ok(key) = FieldKey::new(seg) else { return };
            schema_path = schema_path.join(key);
        }
        // Mirror `collect_non_secret`'s leaf rule: only `Literal` scalars are
        // predicate-addressable. A structured value resolved here (never a
        // secret — secrets are filtered above) is non-addressable.
        if let Some(FieldValue::Literal(v)) = values.get_path(&schema_path) {
            // Key by the same RFC-6901 pointer `PredicateContext::from_json`
            // would have used, so a root predicate written as `/a/b` resolves.
            if let Some(vp) = FieldPath::from_segments(segs) {
                pairs.push((vp, v.clone()));
            }
        }
    });
    PredicateContext::from_fields(pairs)
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
    fn structured_field_with_literal_blob_does_not_leak_nested_secret() {
        // A `Field::Object` whose value is a `Literal` blob (the bypass shape
        // reachable via the public unvalidated `FieldValues::set`) must NOT
        // enter the context — the blob can carry a nested secret's plaintext.
        let obj = Field::object(FieldKey::new("cfg").unwrap())
            .add(Field::secret(FieldKey::new("the_secret").unwrap()));
        let fields = vec![Field::from(obj)];
        let mut values = FieldValues::new();
        values.set(
            FieldKey::new("cfg").unwrap(),
            FieldValue::Literal(json!({ "the_secret": "PLAINTEXT-LEAK" })),
        );
        let ctx = predicate_context_for(&fields, &values);

        assert!(
            ctx.get(&FieldPath::parse("cfg").unwrap()).is_none(),
            "structured-typed field must not contribute a Literal blob"
        );
        assert!(
            ctx.get(&FieldPath::parse("/cfg/the_secret").unwrap())
                .is_none(),
            "nested secret must not be addressable"
        );
        // No plausible pointer yields the plaintext.
        for ptr in ["cfg", "/cfg/the_secret", "the_secret", "/cfg"] {
            if let Some(v) = ctx.get(&FieldPath::parse(ptr).unwrap()) {
                assert!(
                    !v.to_string().contains("PLAINTEXT-LEAK"),
                    "secret plaintext leaked via {ptr}: {v}"
                );
            }
        }
        assert!(
            !format!("{ctx:?}").contains("PLAINTEXT-LEAK"),
            "redacted Debug must never carry the plaintext"
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
