//! Validated schema handles — proof-tokens.

use std::sync::Arc;

use indexmap::IndexMap;
use smallvec::SmallVec;

use crate::{
    error::ValidationError,
    field::Field,
    key::FieldKey,
    path::FieldPath,
    value::{FieldValue, FieldValues},
};

/// Flags computed once at build time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaFlags {
    /// Whether any field allows or requires expressions.
    pub uses_expressions: bool,
    /// Whether any loader-backed field is present (async path).
    pub has_async_loaders: bool,
    /// Maximum nesting depth reached in this schema.
    pub max_depth: u8,
}

/// Cursor into the field tree: breadcrumb of child indices starting from root.
#[derive(Debug, Clone)]
pub struct FieldHandle {
    /// Index path from root fields vec downward.
    pub cursor: SmallVec<[u16; 4]>,
    /// Depth (1 = top-level).
    pub depth: u8,
}

/// Shared interior of a `ValidSchema`.
#[derive(Debug)]
pub struct ValidSchemaInner {
    /// Top-level ordered fields.
    pub fields: Vec<Field>,
    /// Flat index from `FieldPath` → `FieldHandle` for O(1) path lookup.
    pub index: IndexMap<FieldPath, FieldHandle>,
    /// Flags computed during build.
    pub flags: SchemaFlags,
}

/// Proof-token: schema has been built and linted successfully.
///
/// Cheap to clone — backed by `Arc`.
#[derive(Debug, Clone)]
pub struct ValidSchema(pub(crate) Arc<ValidSchemaInner>);

impl ValidSchema {
    pub(crate) fn from_inner(inner: ValidSchemaInner) -> Self {
        Self(Arc::new(inner))
    }

    /// Borrow all top-level fields in insertion order.
    pub fn fields(&self) -> &[Field] {
        &self.0.fields
    }

    /// Borrow the build-time flags.
    pub fn flags(&self) -> &SchemaFlags {
        &self.0.flags
    }

    /// Find a top-level field by key.
    pub fn find(&self, key: &FieldKey) -> Option<&Field> {
        self.0.fields.iter().find(|f| f.key() == key)
    }

    /// Find a field by dotted path using the O(1) index.
    pub fn find_by_path(&self, path: &FieldPath) -> Option<&Field> {
        let handle = self.0.index.get(path)?;
        let mut cur = self.0.fields.get(*handle.cursor.first()? as usize)?;
        for &step in &handle.cursor[1..] {
            cur = match cur {
                Field::Object(o) => o.fields.get(step as usize)?,
                Field::List(l) => l.item.as_deref()?,
                Field::Mode(m) => &m.variants.get(step as usize)?.field,
                _ => return None,
            };
        }
        Some(cur)
    }
}

/// Validated values — tied to a specific `ValidSchema`.
///
/// Produced by `ValidSchema::validate()` (Task 21). Proof-token that values
/// have been checked against the schema at least once.
#[derive(Debug, Clone)]
pub struct ValidValues<'s> {
    pub(crate) schema: &'s ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl<'s> ValidValues<'s> {
    /// Borrow the schema these values were validated against.
    pub fn schema(&self) -> &'s ValidSchema {
        self.schema
    }

    /// Borrow the raw value tree.
    pub fn raw(&self) -> &FieldValues {
        &self.values
    }

    /// Iterate validation warnings that were non-fatal.
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a top-level value by key.
    pub fn get(&self, key: &FieldKey) -> Option<&FieldValue> {
        self.values.get(key)
    }

    /// Look up a value by dotted path.
    pub fn get_path(&self, path: &FieldPath) -> Option<&FieldValue> {
        self.values.get_path(path)
    }
}

/// Resolved values — all `FieldValue::Expression` entries have been evaluated.
///
/// Produced by `ValidValues::resolve()` (Task 23). Proof-token that no
/// expression placeholders remain in the value tree.
#[derive(Debug, Clone)]
pub struct ResolvedValues<'s> {
    pub(crate) schema: &'s ValidSchema,
    pub(crate) values: FieldValues,
    pub(crate) warnings: Arc<[ValidationError]>,
}

impl<'s> ResolvedValues<'s> {
    /// Borrow the schema these values were resolved against.
    pub fn schema(&self) -> &'s ValidSchema {
        self.schema
    }

    /// Iterate resolution warnings.
    pub fn warnings(&self) -> &[ValidationError] {
        &self.warnings
    }

    /// Look up a resolved literal value by key.
    ///
    /// Returns `None` if the field is absent or still an expression
    /// (should not happen in a properly resolved set).
    pub fn get(&self, key: &FieldKey) -> Option<&serde_json::Value> {
        match self.values.get(key)? {
            FieldValue::Literal(v) => Some(v),
            _ => None,
        }
    }

    /// Consume into a flat JSON object.
    pub fn into_json(self) -> serde_json::Value {
        self.values.to_json()
    }

    /// Consume and deserialize into a typed value.
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, Box<ValidationError>> {
        serde_json::from_value(self.into_json()).map_err(|e| {
            Box::new(
                ValidationError::new("type_mismatch")
                    .message(format!("deserialize failed: {e}"))
                    .build(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey, Schema};

    #[test]
    fn clone_is_cheap_via_arc() {
        let s = Schema::builder()
            .add(Field::string(FieldKey::new("x").unwrap()))
            .build()
            .unwrap();
        let c = s.clone();
        assert!(Arc::ptr_eq(&s.0, &c.0));
    }

    #[test]
    fn find_returns_top_level() {
        let s = Schema::builder()
            .add(Field::string(FieldKey::new("x").unwrap()))
            .build()
            .unwrap();
        assert!(s.find(&FieldKey::new("x").unwrap()).is_some());
        assert!(s.find(&FieldKey::new("y").unwrap()).is_none());
    }
}
