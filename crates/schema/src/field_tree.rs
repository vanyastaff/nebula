//! Internal schema-property tree traversal helpers.

use std::collections::HashSet;

use smallvec::SmallVec;

use crate::{
    FieldPath, Property, ValidationError, ValuePath, key::FieldKey, schema::MAX_SCHEMA_DEPTH,
};

pub(crate) type FieldCursor = SmallVec<[u16; 4]>;

/// Reject unsupported declarations before deriving a schema-bound snapshot.
/// Raw properties need a depth proof before the recursive support walk,
/// including mode variants whose keys cannot be represented in the path index.
#[tracing::instrument(level = "debug", skip_all, fields(property_count = properties.len()))]
pub(crate) fn ensure_supported_properties(properties: &[Property]) -> Result<(), ValidationError> {
    let mut pending: Vec<_> = properties.iter().map(|property| (property, 0_u8)).collect();
    while let Some((property, depth)) = pending.pop() {
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
        match property {
            Property::Object(object) => {
                pending.extend(object.fields.iter().map(|child| (child, child_depth)));
            },
            Property::List(list) => {
                pending.extend(list.item.as_deref().map(|item| (item, child_depth)));
            },
            Property::Mode(mode) => {
                pending.extend(
                    mode.variants
                        .iter()
                        .map(|variant| (variant.field.as_ref(), child_depth)),
                );
            },
            Property::String(_)
            | Property::Secret(_)
            | Property::Number(_)
            | Property::Boolean(_)
            | Property::Select(_)
            | Property::Code(_)
            | Property::File(_)
            | Property::Computed(_)
            | Property::Dynamic(_)
            | Property::Notice(_)
            | Property::Unknown(_) => {},
        }
    }
    if let Some(path) = unsupported_property_path(properties) {
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
pub(crate) fn unsupported_property_path(properties: &[Property]) -> Option<ValuePath> {
    properties.iter().find_map(|property| {
        unsupported_property_at(property, ValuePath::root().push(property.key().as_str()))
    })
}

fn unsupported_property_at(property: &Property, path: ValuePath) -> Option<ValuePath> {
    match property {
        Property::Unknown(_) => Some(path),
        Property::Object(object) => object
            .fields
            .iter()
            .find_map(|child| unsupported_property_at(child, path.push(child.key().as_str()))),
        Property::List(list) => list
            .item
            .as_deref()
            .and_then(|item| unsupported_property_at(item, path.push("0"))),
        Property::Mode(mode) => mode
            .variants
            .iter()
            .find_map(|variant| unsupported_property_at(&variant.field, path.push(&variant.key))),
        Property::String(_)
        | Property::Secret(_)
        | Property::Number(_)
        | Property::Boolean(_)
        | Property::Select(_)
        | Property::Code(_)
        | Property::File(_)
        | Property::Computed(_)
        | Property::Dynamic(_)
        | Property::Notice(_) => None,
    }
}

/// A property-like schema node with its canonical schema path and lookup cursor.
#[derive(Debug, Clone)]
pub(crate) struct SchemaNode<'a> {
    pub(crate) property: &'a Property,
    pub(crate) path: FieldPath,
    pub(crate) cursor: FieldCursor,
    pub(crate) depth: u8,
}

/// Walk every indexable property path in depth-first order.
///
/// List items are anonymous, so list-object children are yielded under the
/// list property path (`items.name`), not under an indexed instance path.
/// Mode variants are yielded as synthetic nodes under `mode.variant`.
///
/// Invalid schemas with more than `u16::MAX + 1` siblings are truncated here.
/// Callers that require complete indexing must first run the schema
/// `validate_index_limits` pass and stop on errors.
pub(crate) fn walk_schema_fields<'a>(
    properties: &'a [Property],
    mut visit: impl FnMut(SchemaNode<'a>),
) {
    walk_property_scope(
        properties,
        &FieldPath::root(),
        &FieldCursor::new(),
        0,
        &mut visit,
    );
}

/// Collect every path yielded by [`walk_schema_fields`].
pub(crate) fn defined_field_paths(properties: &[Property]) -> HashSet<FieldPath> {
    let mut defined = HashSet::new();
    walk_schema_fields(properties, |node| {
        defined.insert(node.path);
    });
    defined
}

/// Build a canonical schema path for a mode variant key.
pub(crate) fn mode_variant_path(property_path: &FieldPath, variant_key: &str) -> Option<FieldPath> {
    let key = FieldKey::new(variant_key).ok()?;
    Some(property_path.clone().join(key))
}

fn walk_property_scope<'a>(
    properties: &'a [Property],
    prefix: &FieldPath,
    parent_cursor: &FieldCursor,
    depth: u8,
    visit: &mut impl FnMut(SchemaNode<'a>),
) {
    for (index, property) in properties.iter().enumerate() {
        let Ok(step) = u16::try_from(index) else {
            continue;
        };

        let mut cursor = parent_cursor.clone();
        cursor.push(step);
        let path = prefix.clone().join(property.key().clone());
        let property_depth = depth.saturating_add(1);

        visit(SchemaNode {
            property,
            path: path.clone(),
            cursor: cursor.clone(),
            depth: property_depth,
        });
        walk_property_children(property, &path, &cursor, property_depth, visit);
    }
}

fn walk_property_children<'a>(
    property: &'a Property,
    path: &FieldPath,
    cursor: &FieldCursor,
    depth: u8,
    visit: &mut impl FnMut(SchemaNode<'a>),
) {
    match property {
        Property::Object(object) => {
            walk_property_scope(&object.fields, path, cursor, depth, visit);
        },
        Property::List(list) => {
            if let Some(Property::Object(object)) = list.item.as_deref() {
                let mut item_cursor = cursor.clone();
                // Placeholder step consumed by `ValidSchema::find_by_path` when
                // traversing from `Property::List` to its anonymous item schema.
                item_cursor.push(0);
                walk_property_scope(&object.fields, path, &item_cursor, depth, visit);
            }
        },
        Property::Mode(mode) => {
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
                let variant_property = variant.field.as_ref();

                visit(SchemaNode {
                    property: variant_property,
                    path: variant_path.clone(),
                    cursor: variant_cursor.clone(),
                    depth: variant_depth,
                });
                walk_property_children(
                    variant_property,
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
