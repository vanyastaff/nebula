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
    value::{FieldValue, FieldValues},
};

/// Build a `PredicateContext` from the value tree, recursively, excluding
/// every field declared as `Field::Secret` at any depth.
///
/// Only `FieldValue::Literal` leaves are addressable by predicates;
/// expression / list / mode / secret-sentinel subtrees are non-addressable.
///
/// Reachable across the crate boundary only for seam tests; not a stable
/// public API (`nebula-schema` is pre-1.0 and this is an internal seam).
#[doc(hidden)]
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

/// Build the predicate context for **root rules**.
///
/// Built as the validator's own [`PredicateContext::from_json`] over the
/// submitted JSON with every `Field::Secret` subtree removed by schema type
/// (recursively — through objects, list items, and the selected mode-variant
/// payload). Any field whose schema subtree contains no secret is kept
/// **verbatim**, so the resulting context is byte-identical to the pre-scrub
/// `from_json` there: every legal root predicate (scalar leaf, nested object,
/// whole-object / whole-list presence, array membership, mode envelope)
/// resolves exactly as before and a legitimate guard cannot silently fail
/// open. Only secret plaintext is elided — a `Field::Secret` object key is
/// dropped, a secret list item collapses to `null` (presence/length
/// preserved), the selected mode-variant payload is stripped, and a
/// secret-bearing structured field whose value is not the matching shape is
/// dropped (closes the blob-bypass exfiltration). An undeclared key is kept
/// verbatim only at the **top-level** scope (where it cannot be a secret and
/// no legal predicate can target it — exact `from_json` parity); inside a
/// secret-bearing structured field undeclared keys are attacker-controlled
/// blob keys and are dropped, so a sibling cannot smuggle secret plaintext
/// onto the defined container path. The build-time `secret.predicate_on_value`
/// lint remains the additive outer boundary.
///
/// Reachable across the crate boundary only for seam tests; not a stable
/// public API (`nebula-schema` is pre-1.0 and this is an internal seam).
#[doc(hidden)]
#[must_use]
pub fn root_predicate_context_for(fields: &[Field], values: &FieldValues) -> PredicateContext {
    root_predicate_context_from_json(fields, &values.to_json())
}

/// [`root_predicate_context_for`] over an already-materialized JSON value, so a
/// caller that has built `values.to_json()` does not pay for it twice.
#[must_use]
pub(crate) fn root_predicate_context_from_json(
    fields: &[Field],
    json: &serde_json::Value,
) -> PredicateContext {
    match json.as_object() {
        Some(map) => PredicateContext::from_json(&serde_json::Value::Object(strip_secrets_scope(
            fields, map, true,
        ))),
        None => PredicateContext::from_json(json),
    }
}

/// True when `field`'s schema subtree contains a `Field::Secret` at any depth.
///
/// A pure schema property — independent of the value tree and of any
/// JSON-vs-schema key divergence (mode envelopes use `mode`/`value` JSON keys,
/// not variant keys), so the secret decision is always sound.
///
/// Shared with `loader::redact_secrets_in_value_for_loader` so the loader
/// boundary applies the same blob-bypass defense this module does.
pub(crate) fn field_subtree_has_secret(field: &Field) -> bool {
    match field {
        Field::Secret(_) => true,
        Field::Object(o) => o.fields.iter().any(field_subtree_has_secret),
        Field::List(l) => l.item.as_deref().is_some_and(field_subtree_has_secret),
        Field::Mode(m) => m
            .variants
            .iter()
            .any(|v| field_subtree_has_secret(v.field.as_ref())),
        _ => false,
    }
}

/// Return `value` with every `Field::Secret` subtree removed, guided by
/// `field`'s schema. `None` means the value must not enter the context at all
/// (it *is* a secret, or it is a secret-bearing structured field whose value
/// is the wrong shape — a blob that could smuggle a nested secret's
/// plaintext). A secret-free subtree is returned verbatim, so the later
/// `from_json` keys it exactly as the unscrubbed submission did.
fn strip_secret_value(field: &Field, value: &serde_json::Value) -> Option<serde_json::Value> {
    if matches!(field, Field::Secret(_)) {
        return None; // secret plaintext never enters the context
    }
    if !field_subtree_has_secret(field) {
        return Some(value.clone()); // secret-free → verbatim (`from_json` parity)
    }
    match (field, value) {
        (Field::Object(obj), serde_json::Value::Object(map)) => Some(serde_json::Value::Object(
            // Nested inside a secret-bearing object: drop attacker-controlled
            // undeclared keys so a `Literal`-blob sibling cannot smuggle secret
            // plaintext onto the defined container path.
            strip_secrets_scope(obj.fields.as_slice(), map, false),
        )),
        (Field::List(list), serde_json::Value::Array(items)) => {
            // `from_json` keys the whole array under the list path (it does not
            // descend it). Preserve that — including length / presence so a
            // legal `Set`/`Empty` guard still resolves — with each item's
            // secrets stripped (a secret item collapses to `null`).
            let item = list.item.as_deref();
            let stripped = items
                .iter()
                .map(|el| {
                    item.and_then(|it| strip_secret_value(it, el))
                        .unwrap_or(serde_json::Value::Null)
                })
                .collect();
            Some(serde_json::Value::Array(stripped))
        },
        (Field::Mode(mode), serde_json::Value::Object(env)) => {
            // Keep the `mode` selector (a string, never a secret).
            let mut out = serde_json::Map::new();
            if let Some(sel) = env.get("mode") {
                out.insert("mode".to_owned(), sel.clone());
            }
            // Resolve the active variant exactly as `validate_literal_value`
            // does: an explicit string `mode`, otherwise `default_variant`
            // when `mode` is omitted (a non-string `mode` is an invalid input
            // that fails structural validation — no payload selector then).
            // The payload is stripped against that variant's schema, so a
            // legal root predicate on a default-variant (mode-omitted)
            // submission stays consistent while secrets remain scrubbed.
            let selected: Option<&str> = match env.get("mode") {
                Some(serde_json::Value::String(mk)) => Some(mk.as_str()),
                Some(_) => None,
                None => mode.default_variant.as_deref(),
            };
            if let Some(payload) = env.get("value")
                && let Some(key) = selected
                && let Some(var) = mode.variants.iter().find(|v| v.key.as_str() == key)
                && let Some(stripped) = strip_secret_value(var.field.as_ref(), payload)
            {
                out.insert("value".to_owned(), stripped);
            }
            Some(serde_json::Value::Object(out))
        },
        // Secret-bearing structured field whose value is not the matching
        // shape (e.g. a `Literal` blob handed to an `Object`/`List`/`Mode`
        // field via the public unvalidated setter): drop it — its serialized
        // form could carry a nested secret's plaintext.
        _ => None,
    }
}

/// Strip secrets across one field scope, returning the secret-free object map
/// `from_json` will then key.
///
/// `keep_undeclared` controls keys with no matching schema field. At the
/// **top-level** scope an undeclared key cannot be a `Field::Secret` (secrets
/// are always declared) and a predicate cannot legally target it (the
/// dangling-reference lint rejects undefined paths), so it is kept verbatim
/// for exact `from_json` parity. Inside a **secret-bearing structured field**
/// the keys come from an attacker-controlled `Literal` blob (the unvalidated
/// `FieldValues::set` bypass): an undeclared sibling there can carry secret
/// plaintext that would ride the *defined* container path, so it MUST be
/// dropped. Secret-free subtrees never recurse here (they return verbatim
/// before the match in `strip_secret_value`), so dropping nested undeclared
/// keys never affects the secret-free byte-parity guarantee.
fn strip_secrets_scope(
    schema: &[Field],
    json: &serde_json::Map<String, serde_json::Value>,
    keep_undeclared: bool,
) -> serde_json::Map<String, serde_json::Value> {
    // One lookup per scope instead of a linear scan per JSON key
    // (`strip_secrets_scope` runs at validate-time for every root-rule eval).
    let by_key: std::collections::HashMap<&str, &Field> =
        schema.iter().map(|f| (f.key().as_str(), f)).collect();
    let mut out = serde_json::Map::new();
    for (key, value) in json {
        match by_key.get(key.as_str()) {
            None => {
                if keep_undeclared {
                    out.insert(key.clone(), value.clone());
                }
            },
            Some(field) => {
                if let Some(stripped) = strip_secret_value(field, value) {
                    out.insert(key.clone(), stripped);
                }
            },
        }
    }
    out
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
