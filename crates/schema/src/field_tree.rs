//! Internal schema-field tree traversal helpers.

use std::collections::HashSet;

use smallvec::SmallVec;

use crate::{Field, FieldPath, key::FieldKey};

pub(crate) type FieldCursor = SmallVec<[u16; 4]>;

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
