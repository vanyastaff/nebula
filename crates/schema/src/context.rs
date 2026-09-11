//! Schema-aware, secret-free predicate and loader snapshots.
//!
//! Raw boundaries check depth before traversing or copying input. Projection
//! folds read aliases without cloning authored expressions or secret material.
//! Field and root rules share the same whole-container predicate context.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use nebula_validator::PredicateContext;
use serde_json::Value;

use crate::{
    ValidationError,
    field::{Field, ModeField},
    key::FieldKey,
    secret::SECRET_REDACTED,
    value::{ValuePath, ValueTree},
};

/// Build a secret-free context from unvalidated values, including read aliases.
///
/// Expressions are unavailable. Containers remain addressable after secret
/// subtrees are removed. Unavailable array elements become nulls to preserve positions.
///
/// # Errors
///
/// Returns `recursion_limit` before copying input deeper than the value limit.
#[doc(hidden)]
#[tracing::instrument(level = "debug", skip_all, fields(field_count = fields.len()))]
pub fn predicate_context_for<E>(
    fields: &[Field],
    values: &ValueTree<E>,
) -> Result<PredicateContext, ValidationError> {
    values.check_depth(&ValuePath::root(), 0)?;
    Ok(prepared_predicate_context(fields, values))
}

/// Build the same context for root-rule seam tests over unvalidated values.
///
/// # Errors
///
/// Returns `recursion_limit` before copying input deeper than the value limit.
#[doc(hidden)]
pub fn root_predicate_context_for<E>(
    fields: &[Field],
    values: &ValueTree<E>,
) -> Result<PredicateContext, ValidationError> {
    predicate_context_for(fields, values)
}

/// Build the single field/root rule context after canonical preparation.
///
/// The caller owns the depth proof and supplies pending expression paths to
/// the validator separately. No source text or redaction marker is observable.
#[tracing::instrument(level = "debug", skip_all, fields(field_count = fields.len()))]
pub(crate) fn prepared_predicate_context<E>(
    fields: &[Field],
    values: &ValueTree<E>,
) -> PredicateContext {
    PredicateContext::from_json(project_root(fields, values, Projection::Predicates))
}

/// Materialize a bounded loader snapshot without exposing expression sources.
pub(crate) fn redacted_loader_json<E>(
    fields: &[Field],
    values: &ValueTree<E>,
) -> Result<Value, ValidationError> {
    values.check_depth(&ValuePath::root(), 0)?;
    Ok(project_root(fields, values, Projection::Loader))
}

#[derive(Clone, Copy)]
enum Projection {
    Predicates,
    Loader,
}

impl Projection {
    fn unavailable(self) -> Option<Value> {
        match self {
            Self::Predicates => None,
            Self::Loader => Some(Value::String(SECRET_REDACTED.to_owned())),
        }
    }

    fn keeps_undeclared(self, secret_bearing: bool) -> bool {
        !secret_bearing || matches!(self, Self::Loader)
    }
}

fn project_root<E>(fields: &[Field], values: &ValueTree<E>, projection: Projection) -> Value {
    if let ValueTree::Object(values) = values {
        Value::Object(project_scope(fields, values, projection, false))
    } else if fields.iter().any(field_subtree_has_secret) {
        projection.unavailable().unwrap_or(Value::Null)
    } else {
        project_value(None, values, projection).unwrap_or(Value::Null)
    }
}

/// Inspect raw schema trees without recursion, including inactive mode variants.
pub(crate) fn field_subtree_has_secret(field: &Field) -> bool {
    let mut pending = vec![field];
    while let Some(field) = pending.pop() {
        match field {
            Field::Secret(_) => return true,
            Field::Object(object) => pending.extend(&object.fields),
            Field::List(list) => pending.extend(list.item.as_deref()),
            Field::Mode(mode) => {
                pending.extend(mode.variants.iter().map(|variant| variant.field.as_ref()));
            },
            _ => {},
        }
    }
    false
}

fn project_value<E>(
    field: Option<&Field>,
    value: &ValueTree<E>,
    projection: Projection,
) -> Option<Value> {
    if matches!(field, Some(Field::Secret(_)))
        || matches!(value, ValueTree::Secret(_) | ValueTree::Expression(_))
    {
        return projection.unavailable();
    }

    let secret_bearing = field.is_some_and(field_subtree_has_secret);
    match (field, value) {
        (Some(Field::Object(object)), ValueTree::Object(values)) => Some(Value::Object(
            project_scope(&object.fields, values, projection, secret_bearing),
        )),
        (Some(Field::List(list)), ValueTree::List(values)) => Some(Value::Array(
            values
                .iter()
                .map(|value| {
                    project_value(list.item.as_deref(), value, projection).unwrap_or(Value::Null)
                })
                .collect(),
        )),
        (Some(Field::Mode(mode)), ValueTree::Object(values)) => Some(Value::Object(project_mode(
            mode,
            values,
            projection,
            secret_bearing,
        ))),
        (Some(Field::Object(_) | Field::List(_) | Field::Mode(_)), _) if secret_bearing => {
            projection.unavailable()
        },
        (_, ValueTree::Literal(value)) => Some(value.as_json().clone()),
        (_, ValueTree::Object(values)) => Some(Value::Object(
            values
                .iter()
                .filter_map(|(key, value)| {
                    project_value(None, value, projection).map(|value| (key.clone(), value))
                })
                .collect(),
        )),
        (_, ValueTree::List(values)) => Some(Value::Array(
            values
                .iter()
                .map(|value| project_value(None, value, projection).unwrap_or(Value::Null))
                .collect(),
        )),
        (_, ValueTree::Expression(_) | ValueTree::Secret(_)) => projection.unavailable(),
    }
}

/// Select canonical input first, otherwise the first declared alias. Losing
/// aliases are never copied, even when their values have a different shape.
fn project_scope<E>(
    fields: &[Field],
    values: &IndexMap<String, ValueTree<E>>,
    projection: Projection,
    secret_bearing: bool,
) -> serde_json::Map<String, Value> {
    let by_key: HashMap<&str, &Field> = fields
        .iter()
        .map(|field| (field.key().as_str(), field))
        .collect();
    let aliases: HashSet<&str> = fields
        .iter()
        .flat_map(|field| field.read_aliases().iter().map(FieldKey::as_str))
        .collect();
    let mut output = serde_json::Map::new();
    for (key, value) in values {
        let field = by_key.get(key.as_str()).copied();
        if field.is_none()
            && (aliases.contains(key.as_str()) || !projection.keeps_undeclared(secret_bearing))
        {
            continue;
        }
        if let Some(value) = project_value(field, value, projection) {
            output.insert(key.clone(), value);
        }
    }
    for field in fields {
        if values.contains_key(field.key().as_str()) {
            continue;
        }
        if let Some(value) = field
            .read_aliases()
            .iter()
            .find_map(|alias| values.get(alias.as_str()))
            .and_then(|value| project_value(Some(field), value, projection))
        {
            output.insert(field.key().as_str().to_owned(), value);
        }
    }
    output
}

fn project_mode<E>(
    mode: &ModeField,
    values: &IndexMap<String, ValueTree<E>>,
    projection: Projection,
    secret_bearing: bool,
) -> serde_json::Map<String, Value> {
    let selected = match values.get("mode") {
        Some(value) => value.as_str(),
        None => mode.default_variant.as_deref(),
    };
    let variant = selected.and_then(|key| mode.variants.iter().find(|variant| variant.key == key));
    values
        .iter()
        .filter_map(|(key, value)| {
            let projected = match key.as_str() {
                "mode" if value.as_str().is_some() => project_value(None, value, projection),
                "mode" => projection.unavailable(),
                "value" => match variant {
                    Some(variant) => project_value(Some(&variant.field), value, projection),
                    None => projection.unavailable(),
                },
                _ if projection.keeps_undeclared(secret_bearing) => {
                    project_value(None, value, projection)
                },
                _ => None,
            };
            projected.map(|value| (key.clone(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{AuthoredValue, ScalarValue, field_key};

    #[test]
    fn literal_field_is_visible_to_predicates() {
        let fields = vec![Field::from(Field::string(field_key!("name")))];
        let values = AuthoredValue::from_data(json!({"name": "alice"})).unwrap();
        let ctx = predicate_context_for(&fields, &values).unwrap();
        assert_eq!(
            ctx.get(&ValuePath::parse("/name").unwrap()),
            Some(&json!("alice"))
        );
    }

    #[test]
    fn pre_resolve_plaintext_secret_is_scrubbed_by_schema_type() {
        // A Field::Secret holding a pre-resolve plaintext Literal MUST NOT
        // enter the predicate context. The old runtime-tag scrub failed this.
        let fields = vec![Field::from(Field::secret(field_key!("api_key")))];
        let values = AuthoredValue::from_data(json!({"api_key": "s3cr3t-plaintext"})).unwrap();
        let ctx = predicate_context_for(&fields, &values).unwrap();
        assert!(
            ctx.get(&ValuePath::parse("/api_key").unwrap()).is_none(),
            "secret-typed field must be excluded from the predicate context"
        );
    }

    #[test]
    fn structured_field_with_literal_blob_does_not_leak_nested_secret() {
        // Container literals are unrepresentable. A scalar containing serialized
        // secret data is still a wrong shape and must be excluded.
        let error = ScalarValue::try_from(json!({"the_secret": "PLAINTEXT-LEAK"})).unwrap_err();
        assert_eq!(error.code(), "type_mismatch");
        let obj = Field::object(field_key!("cfg")).add(Field::secret(field_key!("the_secret")));
        let fields = vec![Field::from(obj)];
        let values = AuthoredValue::from_data(json!({
            "cfg": json!({"the_secret": "PLAINTEXT-LEAK"}).to_string()
        }))
        .unwrap();
        let ctx = predicate_context_for(&fields, &values).unwrap();

        assert!(
            ctx.get(&ValuePath::parse("/cfg").unwrap()).is_none(),
            "structured-typed field must not contribute a Literal blob"
        );
        assert!(
            ctx.get(&ValuePath::parse("/cfg/the_secret").unwrap())
                .is_none(),
            "nested secret must not be addressable"
        );
        // No plausible pointer yields the plaintext.
        for ptr in ["/cfg", "/cfg/the_secret", "/the_secret"] {
            if let Some(v) = ctx.get(&ValuePath::parse(ptr).unwrap()) {
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
        let fields = vec![Field::from(Field::string(field_key!("region")))];
        let values = AuthoredValue::from_data(json!({"region": "eu-secret-marker"})).unwrap();
        let ctx = predicate_context_for(&fields, &values).unwrap();
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
