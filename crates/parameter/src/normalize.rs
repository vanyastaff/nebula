//! Default and mode normalization helpers.
//!
//! Applies schema defaults and mode default-variant backfilling to a
//! [`FieldValues`] map without mutating user-supplied values.

use crate::field::Field;
use crate::values::FieldValues;

/// Applies schema defaults to `values` for each field in `fields`.
///
/// Existing user-provided values are preserved. Missing fields are
/// materialized from `default` metadata and mode default variants.
#[must_use]
pub fn normalize_fields(fields: &[Field], values: &FieldValues) -> FieldValues {
    let mut normalized = values.clone();

    for field in fields {
        let field_id = field.meta().id.clone();
        let current = normalized.get(&field_id).cloned();

        if let Some(next_value) = normalize_field_value(field, current) {
            normalized.set(field_id, next_value);
        }
    }

    normalized
}

pub(crate) fn normalize_field_value(
    field: &Field,
    current: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut value = match current {
        Some(value) => value,
        None => default_field_value(field)?,
    };

    if let Field::Mode {
        variants,
        default_variant,
        ..
    } = field
        && let Some(object) = value.as_object_mut()
    {
        let selected_mode = object
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .or_else(|| default_variant.clone());

        if let Some(selected_mode) = selected_mode {
            object
                .entry("mode".to_owned())
                .or_insert_with(|| serde_json::Value::String(selected_mode.clone()));

            if let Some(variant) = variants.iter().find(|variant| variant.key == selected_mode) {
                let nested = object.get("value").cloned();
                if let Some(nested_normalized) = normalize_field_value(&variant.content, nested) {
                    object.insert("value".to_owned(), nested_normalized);
                }
            }
        }
    }

    Some(value)
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
