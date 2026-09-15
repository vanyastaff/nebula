//! Internal schema-field tree traversal helpers.

use std::collections::HashSet;

use smallvec::SmallVec;

use crate::{
    Field, FieldPath, ValidationError, ValuePath, key::FieldKey, schema::MAX_SCHEMA_DEPTH,
};

pub(crate) type FieldCursor = SmallVec<[u16; 4]>;

/// Reject unsupported declarations before deriving a schema-bound snapshot.
/// Raw fields need a depth proof before the recursive support walk, including
/// mode variants whose keys cannot be represented in the field index.
#[tracing::instrument(level = "debug", skip_all, fields(field_count = fields.len()))]
pub(crate) fn ensure_supported_properties(fields: &[Field]) -> Result<(), ValidationError> {
    let mut pending: Vec<_> = fields.iter().map(|field| (field, 0_u8)).collect();
    while let Some((field, depth)) = pending.pop() {
        if depth > MAX_SCHEMA_DEPTH {
            tracing::debug!(
                code = "schema.depth_limit",
                "snapshot schema exceeds depth limit"
            );
            return Err(ValidationError::builder("schema.depth_limit")
                .param("limit", MAX_SCHEMA_DEPTH)
                .message("schema nesting depth exceeds the supported limit")
                .build());
        }
        let child_depth = depth.saturating_add(1);
        match field {
            Field::Object(object) => {
                pending.extend(object.fields.iter().map(|child| (child, child_depth)));
            },
            Field::List(list) => {
                pending.extend(list.item.as_deref().map(|item| (item, child_depth)));
            },
            Field::Mode(mode) => {
                pending.extend(
                    mode.variants
                        .iter()
                        .map(|variant| (variant.field.as_ref(), child_depth)),
                );
            },
            Field::String(_)
            | Field::Secret(_)
            | Field::Number(_)
            | Field::Boolean(_)
            | Field::Select(_)
            | Field::Code(_)
            | Field::File(_)
            | Field::Computed(_)
            | Field::Dynamic(_)
            | Field::Notice(_)
            | Field::Unknown(_) => {},
        }
    }
    if let Some(path) = unsupported_property_path(fields) {
        tracing::debug!(code = "schema.unsupported_property_kind", %path,
            "unsupported declaration rejected before snapshot projection");
        return Err(ValidationError::builder("schema.unsupported_property_kind")
            .at(path)
            .message("schema contains an unsupported property kind")
            .build());
    }
    Ok(())
}

/// Inspect every declaration, including anonymous items and inactive variants.
/// Callers pass an admitted, depth-bounded schema; indexing alone omits scalar items.
pub(crate) fn unsupported_property_path(fields: &[Field]) -> Option<ValuePath> {
    fields.iter().find_map(|field| {
        unsupported_property_at(field, ValuePath::root().push(field.key().as_str()))
    })
}

fn unsupported_property_at(field: &Field, path: ValuePath) -> Option<ValuePath> {
    match field {
        Field::Unknown(_) => Some(path),
        Field::Object(object) => object
            .fields
            .iter()
            .find_map(|child| unsupported_property_at(child, path.push(child.key().as_str()))),
        Field::List(list) => list
            .item
            .as_deref()
            .and_then(|item| unsupported_property_at(item, path.push("0"))),
        Field::Mode(mode) => mode
            .variants
            .iter()
            .find_map(|variant| unsupported_property_at(&variant.field, path.push(&variant.key))),
        Field::String(_)
        | Field::Secret(_)
        | Field::Number(_)
        | Field::Boolean(_)
        | Field::Select(_)
        | Field::Code(_)
        | Field::File(_)
        | Field::Computed(_)
        | Field::Dynamic(_)
        | Field::Notice(_) => None,
    }
}

/// A field-like schema node with its canonical schema path and lookup cursor.
#[derive(Debug, Clone)]
pub(crate) struct SchemaNode<'a> {
    pub(crate) field: &'a Field,
    pub(crate) path: FieldPath,
    pub(crate) cursor: FieldCursor,
    pub(crate) depth: u8,
}

/// Walk every indexable field path in depth-first order.
///
/// List items are anonymous, so list-object children are yielded under the
/// list field path (`items.name`), not under an indexed instance path.
/// Mode variants are yielded as synthetic nodes under `mode.variant`.
///
/// Invalid schemas with more than `u16::MAX + 1` siblings are truncated here.
/// Callers that require complete indexing must first run the schema
/// `validate_index_limits` pass and stop on errors.
pub(crate) fn walk_schema_fields<'a>(fields: &'a [Field], mut visit: impl FnMut(SchemaNode<'a>)) {
    walk_field_scope(
        fields,
        &FieldPath::root(),
        &FieldCursor::new(),
        0,
        &mut visit,
    );
}

/// Collect every path yielded by [`walk_schema_fields`].
pub(crate) fn defined_field_paths(fields: &[Field]) -> HashSet<FieldPath> {
    let mut defined = HashSet::new();
    walk_schema_fields(fields, |node| {
        defined.insert(node.path);
    });
    defined
}

/// Build a canonical schema path for a mode variant key.
pub(crate) fn mode_variant_path(field_path: &FieldPath, variant_key: &str) -> Option<FieldPath> {
    let key = FieldKey::new(variant_key).ok()?;
    Some(field_path.clone().join(key))
}

fn walk_field_scope<'a>(
    fields: &'a [Field],
    prefix: &FieldPath,
    parent_cursor: &FieldCursor,
    depth: u8,
    visit: &mut impl FnMut(SchemaNode<'a>),
) {
    for (index, field) in fields.iter().enumerate() {
        let Ok(step) = u16::try_from(index) else {
            continue;
        };

        let mut cursor = parent_cursor.clone();
        cursor.push(step);
        let path = prefix.clone().join(field.key().clone());
        let field_depth = depth.saturating_add(1);

        visit(SchemaNode {
            field,
            path: path.clone(),
            cursor: cursor.clone(),
            depth: field_depth,
        });
        walk_field_children(field, &path, &cursor, field_depth, visit);
    }
}

fn walk_field_children<'a>(
    field: &'a Field,
    path: &FieldPath,
    cursor: &FieldCursor,
    depth: u8,
    visit: &mut impl FnMut(SchemaNode<'a>),
) {
    match field {
        Field::Object(object) => {
            walk_field_scope(&object.fields, path, cursor, depth, visit);
        },
        Field::List(list) => {
            if let Some(Field::Object(object)) = list.item.as_deref() {
                let mut item_cursor = cursor.clone();
                // Placeholder step consumed by `ValidSchema::find_by_path` when
                // traversing from `Field::List` to its anonymous item schema.
                item_cursor.push(0);
                walk_field_scope(&object.fields, path, &item_cursor, depth, visit);
            }
        },
        Field::Mode(mode) => {
            for (variant_index, variant) in mode.variants.iter().enumerate() {
                let Some(variant_path) = mode_variant_path(path, variant.key.as_str()) else {
                    continue;
                };
                let Ok(step) = u16::try_from(variant_index) else {
                    continue;
                };

                let mut variant_cursor = cursor.clone();
                variant_cursor.push(step);
                let variant_depth = depth.saturating_add(1);
                let variant_field = variant.field.as_ref();

                visit(SchemaNode {
                    field: variant_field,
                    path: variant_path.clone(),
                    cursor: variant_cursor.clone(),
                    depth: variant_depth,
                });
                walk_field_children(
                    variant_field,
                    &variant_path,
                    &variant_cursor,
                    variant_depth,
                    visit,
                );
            }
        },
        _ => {},
    }
}
