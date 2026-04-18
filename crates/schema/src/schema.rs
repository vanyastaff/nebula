//! Schema container and builder.
//!
//! `SchemaBuilder::build()` runs structural lint passes and produces a
//! `ValidSchema` proof-token.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smallvec::SmallVec;

use crate::{
    Field, LoaderContext, LoaderRegistry, LoaderResult, SelectOption,
    error::{ValidationError, ValidationReport},
    path::FieldPath,
    validated::{FieldHandle, SchemaFlags, ValidSchema, ValidSchemaInner},
};

// ── Builder entry point ───────────────────────────────────────────────────────

/// Schema aggregate — a collection of typed field definitions.
///
/// Build a schema with `Schema::builder()` then call `SchemaBuilder::build()`
/// to get a `ValidSchema` proof-token.
///
/// # Example
///
/// ```rust
/// use nebula_schema::{Field, Schema, field_key};
///
/// let schema = Schema::builder()
///     .add(Field::string(field_key!("name")).required())
///     .add(Field::number(field_key!("score")))
///     .build()
///     .expect("valid schema");
///
/// assert_eq!(schema.fields().len(), 2);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Ordered field list.
    fields: Vec<Field>,
}

impl Schema {
    /// Create a new `SchemaBuilder`.
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }

    /// Create an empty schema.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add field and return updated schema.
    ///
    /// If a field with the same key already exists it is replaced.
    #[expect(
        clippy::should_implement_trait,
        reason = "builder API mirrors existing add-style schema DSL"
    )]
    pub fn add(mut self, field: impl Into<Field>) -> Self {
        let field = field.into();
        let key = field.key().as_str();
        if let Some(existing) = self
            .fields
            .iter_mut()
            .find(|existing| existing.key().as_str() == key)
        {
            *existing = field;
        } else {
            self.fields.push(field);
        }
        self
    }

    /// Number of top-level fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns true when schema has no fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Find field by key (string slice).
    pub fn find(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.key().as_str() == key)
    }

    /// Borrow all top-level fields in insertion order.
    pub fn fields(&self) -> &[Field] {
        self.fields.as_slice()
    }

    /// Run static lint checks for schema structure and references.
    ///
    /// Returns a [`ValidationReport`] — warnings are advisory, errors indicate
    /// structural problems.
    pub fn lint(&self) -> ValidationReport {
        let mut report = ValidationReport::new();
        crate::lint::lint_tree(&self.fields, &FieldPath::root(), &mut report);
        report
    }

    /// Resolve dynamic options for a select field through loader registry.
    pub async fn load_select_options(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let field = self.find(key).ok_or_else(|| {
            ValidationError::builder("loader.not_registered")
                .message(format!("field `{key}` not found in schema"))
                .build()
        })?;
        let Field::Select(select) = field else {
            return Err(ValidationError::builder("loader.not_registered")
                .message(format!(
                    "field `{key}` is not a select field (got {})",
                    field.type_name()
                ))
                .build());
        };
        let Some(loader_key) = select.loader.as_deref() else {
            return Err(ValidationError::builder("loader.not_registered")
                .message(format!("field `{key}` has no loader configured"))
                .build());
        };
        registry.load_options(loader_key, context).await
    }

    /// Resolve dynamic record payloads for a dynamic field through registry.
    pub async fn load_dynamic_records(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let field = self.find(key).ok_or_else(|| {
            ValidationError::builder("loader.not_registered")
                .message(format!("field `{key}` not found in schema"))
                .build()
        })?;
        let Field::Dynamic(dynamic) = field else {
            return Err(ValidationError::builder("loader.not_registered")
                .message(format!(
                    "field `{key}` is not a dynamic field (got {})",
                    field.type_name()
                ))
                .build());
        };
        let Some(loader_key) = dynamic.loader.as_deref() else {
            return Err(ValidationError::builder("loader.not_registered")
                .message(format!("field `{key}` has no loader configured"))
                .build());
        };
        registry.load_records(loader_key, context).await
    }
}

// ── SchemaBuilder ─────────────────────────────────────────────────────────────

/// Mutable builder state. Consumed by `build()`.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    fields: Vec<Field>,
}

impl SchemaBuilder {
    /// Append a field to the builder.
    #[expect(
        clippy::should_implement_trait,
        reason = "builder API mirrors add-style schema DSL"
    )]
    pub fn add(mut self, field: impl Into<Field>) -> Self {
        self.fields.push(field.into());
        self
    }

    /// Append many fields at once — accepts `Vec<Field>`, `[Field; N]`,
    /// iterators, and anything `Into<Field>` per item. Preferred over
    /// chaining `.add(...)` for statically known bulk additions.
    #[must_use]
    pub fn add_many<I, F>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = F>,
        F: Into<Field>,
    {
        self.fields.extend(fields.into_iter().map(Into::into));
        self
    }

    /// Append a group of fields that share a common label and optional
    /// `visible_when` / `required_when` conditions.
    ///
    /// ```rust
    /// use nebula_schema::{FieldCollector, Schema, StringWidget};
    /// use nebula_validator::{Predicate, Rule};
    ///
    /// let rule = Rule::predicate(Predicate::eq("method", "POST").unwrap());
    /// let schema = Schema::builder()
    ///     .string("method", |s| s.required())
    ///     .group("body_section", |g| {
    ///         g.visible_when(rule)
    ///             .string("body", |s| s.widget(StringWidget::Multiline))
    ///     })
    ///     .build()
    ///     .unwrap();
    /// assert_eq!(schema.fields().len(), 2);
    /// ```
    #[must_use]
    pub fn group(
        mut self,
        name: impl Into<String>,
        f: impl FnOnce(crate::builder::GroupBuilder) -> crate::builder::GroupBuilder,
    ) -> Self {
        let builder = f(crate::builder::GroupBuilder::new(name));
        self.fields.extend(builder.into_fields());
        self
    }

    /// Borrow the fields currently staged on the builder.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Run lint passes and produce a validated schema, or a report of errors.
    pub fn build(self) -> Result<ValidSchema, ValidationReport> {
        let mut report = ValidationReport::new();

        // Lint passes (Task 19 fills these out fully).
        crate::lint::lint_tree(&self.fields, &FieldPath::root(), &mut report);

        if report.has_errors() {
            return Err(report);
        }

        // Build the flat path index for O(1) path lookup.
        let mut index: IndexMap<FieldPath, FieldHandle> = IndexMap::new();
        let mut flags = SchemaFlags::default();
        build_index(
            &self.fields,
            &FieldPath::root(),
            SmallVec::new(),
            0,
            &mut index,
            &mut flags,
        );

        Ok(ValidSchema::from_inner(ValidSchemaInner {
            fields: self.fields,
            index,
            flags,
        }))
    }
}

impl crate::builder::FieldCollector for SchemaBuilder {
    fn push_field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }
}

// ── Index builder ─────────────────────────────────────────────────────────────

fn build_index(
    fields: &[Field],
    prefix: &FieldPath,
    parent_cursor: SmallVec<[u16; 4]>,
    depth: u8,
    index: &mut IndexMap<FieldPath, FieldHandle>,
    flags: &mut SchemaFlags,
) {
    use crate::mode::ExpressionMode;

    for (i, f) in fields.iter().enumerate() {
        let mut cursor = parent_cursor.clone();
        cursor.push(i as u16);
        let path = prefix.clone().join(f.key().clone());
        flags.max_depth = flags.max_depth.max(depth + 1);

        // Track expression usage.
        if !matches!(f.expression(), ExpressionMode::Forbidden) {
            flags.uses_expressions = true;
        }

        // Track async loader usage.
        let has_loader = match f {
            Field::Select(s) => s.loader.is_some(),
            Field::Dynamic(d) => d.loader.is_some(),
            _ => false,
        };
        if has_loader {
            flags.has_async_loaders = true;
        }

        index.insert(
            path.clone(),
            FieldHandle {
                cursor: cursor.clone(),
                depth: depth + 1,
            },
        );

        // Recurse for container types.
        match f {
            Field::Object(obj) => {
                build_index(&obj.fields, &path, cursor, depth + 1, index, flags);
            },
            Field::List(list) => {
                if let Some(item) = list.item.as_deref() {
                    // Index the item schema itself under the list path.
                    let mut child_cursor = cursor.clone();
                    child_cursor.push(0);
                    let item_path = path.clone().join(f.key().clone());
                    // If item is an object, recurse into its fields.
                    if let Field::Object(o) = item {
                        build_index(&o.fields, &path, cursor, depth + 1, index, flags);
                    }
                    let _ = item_path; // suppress unused warning
                }
            },
            Field::Mode(mode) => {
                index_mode_variants(mode, &path, &cursor, depth, index, flags);
            },
            _ => {},
        }
    }
}

fn index_mode_variants(
    mode: &crate::field::ModeField,
    path: &FieldPath,
    parent_cursor: &SmallVec<[u16; 4]>,
    depth: u8,
    index: &mut IndexMap<FieldPath, FieldHandle>,
    flags: &mut SchemaFlags,
) {
    for (vi, variant) in mode.variants.iter().enumerate() {
        let Ok(vk) = crate::key::FieldKey::new(variant.key.as_str()) else {
            continue;
        };
        let mut v_cursor = parent_cursor.clone();
        v_cursor.push(vi as u16);
        let variant_path = path.clone().join(vk);
        index.insert(
            variant_path.clone(),
            FieldHandle {
                cursor: v_cursor.clone(),
                depth: depth + 2,
            },
        );
        if let Field::Object(o) = variant.field.as_ref() {
            build_index(&o.fields, &variant_path, v_cursor, depth + 2, index, flags);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey};

    fn fk(s: &str) -> FieldKey {
        FieldKey::new(s).unwrap()
    }

    #[test]
    fn build_empty_schema_ok() {
        let s = Schema::builder().build().unwrap();
        assert_eq!(s.fields().len(), 0);
    }

    #[test]
    fn build_detects_duplicate_key() {
        let r = Schema::builder()
            .add(Field::string(fk("x")))
            .add(Field::number(fk("x")))
            .build();
        let err = r.unwrap_err();
        assert!(err.errors().any(|e| e.code == "duplicate_key"));
    }

    #[test]
    fn build_finds_field_by_key() {
        let s = Schema::builder()
            .add(Field::string(fk("a")))
            .build()
            .unwrap();
        let key = FieldKey::new("a").unwrap();
        assert!(s.find(&key).is_some());
    }

    #[test]
    fn schema_flags_track_depth() {
        let s = Schema::builder()
            .add(Field::string(fk("a")))
            .add(Field::number(fk("b")))
            .build()
            .unwrap();
        assert_eq!(s.flags().max_depth, 1);
    }

    #[test]
    fn legacy_new_add_still_compiles() {
        let schema = Schema::new().add(Field::string(fk("x")));
        assert_eq!(schema.len(), 1);
    }
}
