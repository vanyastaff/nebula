//! Default and mode normalization helpers.
//!
//! Applies schema defaults and mode default-variant backfilling to a
//! [`FieldValues`] map without mutating user-supplied values.
//!
//! Recursion into nested [`Field::Object`] and [`Field::List`] fields is
//! bounded by [`MAX_NORMALIZE_DEPTH`] to prevent stack overflows from
//! deeply nested or self-referencing schemas.

use crate::field::Field;
use crate::values::FieldValues;

/// Maximum recursion depth for nested normalization.
const MAX_NORMALIZE_DEPTH: u8 = 16;

/// Applies schema defaults to `values` for each field in `fields`.
///
/// Existing user-provided values are preserved. Missing fields are
/// materialized from `default` metadata and mode default variants.
/// Nested [`Field::Object`] and [`Field::List`] fields are recursed into.
#[must_use]
pub fn normalize_fields(fields: &[Field], values: &FieldValues) -> FieldValues {
    let mut normalized = values.clone();

    for field in fields {
        let field_id = field.meta().id.clone();
        let current = normalized.get(&field_id).cloned();

        if let Some(next_value) = normalize_field_value(field, current, 0) {
            normalized.set(field_id, next_value);
        }
    }

    normalized
}

pub(crate) fn normalize_field_value(
    field: &Field,
    current: Option<serde_json::Value>,
    depth: u8,
) -> Option<serde_json::Value> {
    if depth >= MAX_NORMALIZE_DEPTH {
        return current;
    }

    let mut value = match current {
        Some(value) => value,
        None => default_field_value(field)?,
    };

    match field {
        Field::Mode {
            variants,
            default_variant,
            ..
        } => {
            normalize_mode(&mut value, variants, default_variant.as_deref(), depth);
        }
        Field::Object { fields, .. } => {
            if let Some(object) = value.as_object_mut() {
                for nested in fields {
                    let nested_id = &nested.meta().id;
                    let nested_current = object.get(nested_id).cloned();

                    if let Some(normalized) =
                        normalize_field_value(nested, nested_current, depth + 1)
                    {
                        object.insert(nested_id.clone(), normalized);
                    }
                }
            }
        }
        Field::List { item, .. } => {
            if let Some(items) = value.as_array_mut() {
                for item_value in items.iter_mut() {
                    if let Some(normalized) =
                        normalize_field_value(item, Some(item_value.clone()), depth + 1)
                    {
                        *item_value = normalized;
                    }
                }
            }
        }
        _ => {}
    }

    Some(value)
}

fn normalize_mode(
    value: &mut serde_json::Value,
    variants: &[crate::spec::ModeVariant],
    default_variant: Option<&str>,
    depth: u8,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let selected_mode = object
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| default_variant.map(str::to_owned));

    let Some(selected_mode) = selected_mode else {
        return;
    };

    object
        .entry("mode".to_owned())
        .or_insert_with(|| serde_json::Value::String(selected_mode.clone()));

    let Some(variant) = variants.iter().find(|v| v.key == selected_mode) else {
        return;
    };

    let nested = object.get("value").cloned();
    if let Some(normalized) = normalize_field_value(&variant.content, nested, depth + 1) {
        object.insert("value".to_owned(), normalized);
    }
}

fn default_field_value(field: &Field) -> Option<serde_json::Value> {
    let meta_default = field.meta().default.clone();
    if meta_default.is_some() {
        return meta_default;
    }

    if let Field::Mode {
        default_variant, ..
    } = field
        && let Some(default_variant) = default_variant
    {
        let mut object = serde_json::Map::new();
        object.insert(
            "mode".to_owned(),
            serde_json::Value::String(default_variant.clone()),
        );
        return Some(serde_json::Value::Object(object));
    }

    None
}
