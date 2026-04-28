//! Schema container and builder.
//!
//! `SchemaBuilder::build()` runs structural lint passes and produces a
//! `ValidSchema` proof-token.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use smallvec::SmallVec;

use crate::{
    Field, LoaderContext, LoaderRegistry, LoaderResult, SelectOption,
    error::{ValidationError, ValidationReport},
    path::{FieldPath, PathSegment},
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
    #[must_use]
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder::default()
    }

    /// Number of top-level fields.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns true when schema has no fields.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Find field by key (string slice).
    #[must_use]
    pub fn find(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.key().as_str() == key)
    }

    /// Borrow all top-level fields in insertion order.
    #[must_use]
    pub const fn fields(&self) -> &[Field] {
        self.fields.as_slice()
    }

    /// Run static lint checks for schema structure and references.
    ///
    /// Returns a [`ValidationReport`] — warnings are advisory, errors indicate
    /// structural problems.
    #[must_use]
    pub fn lint(&self) -> ValidationReport {
        let mut report = ValidationReport::new();
        crate::lint::lint_tree(&self.fields, &FieldPath::root(), &mut report);
        report
    }

    /// Resolve dynamic options for a select field through loader registry.
    ///
    /// # Errors
    ///
    /// - `field.not_found` — schema has no field with this key.
    /// - `field.type_mismatch` — field exists but isn't a `Select`. Carries `expected` and `actual`
    ///   params.
    /// - `loader.missing_config` — field is a select but has no loader configured (static options
    ///   only).
    /// - `loader.not_registered` / `loader.failed` — propagated from
    ///   [`LoaderRegistry::load_options`].
    pub async fn load_select_options(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let path = FieldPath::root().join(parse_top_level_key(key)?);
        self.load_select_options_at(&path, registry, context).await
    }

    /// Resolve dynamic options for a select field at a nested schema path.
    ///
    /// This path addresses the schema tree, not a concrete runtime instance.
    /// For example:
    ///
    /// - nested object child: `config.workspace`
    /// - list item child: `rows[0].workspace` or `rows.workspace`
    /// - mode variant child: `auth.oauth.workspace`
    ///
    /// # Errors
    ///
    /// - `field.not_found` — schema has no field at this path.
    /// - `field.type_mismatch` — field exists but isn't a `Select`. Carries `expected` and `actual`
    ///   params.
    /// - `loader.missing_config` — field is a select but has no loader configured (static options
    ///   only).
    /// - `loader.not_registered` / `loader.failed` — propagated from
    ///   [`LoaderRegistry::load_options`].
    pub async fn load_select_options_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let loader_key = resolve_select_loader_path(self.fields(), path)?;
        registry.load_options(&loader_key, context).await
    }

    /// Resolve dynamic record payloads for a dynamic field through registry.
    ///
    /// # Errors
    ///
    /// Same taxonomy as [`Schema::load_select_options`] — `field.not_found`,
    /// `field.type_mismatch`, `loader.missing_config`, or the registry's
    /// `loader.not_registered` / `loader.failed`.
    pub async fn load_dynamic_records(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let path = FieldPath::root().join(parse_top_level_key(key)?);
        self.load_dynamic_records_at(&path, registry, context).await
    }

    /// Resolve dynamic record payloads for a field at a nested schema path.
    ///
    /// Uses the same schema-path addressing rules as [`Schema::load_select_options_at`].
    ///
    /// # Errors
    ///
    /// Same taxonomy as [`Schema::load_select_options_at`] — `field.not_found`,
    /// `field.type_mismatch`, `loader.missing_config`, or the registry's
    /// `loader.not_registered` / `loader.failed`.
    pub async fn load_dynamic_records_at(
        &self,
        path: &FieldPath,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let loader_key = resolve_dynamic_loader_path(self.fields(), path)?;
        registry.load_records(&loader_key, context).await
    }
}

// ── Loader resolution helpers ─────────────────────────────────────────────────

/// Parse a top-level key for loader APIs.
///
/// # Errors
///
/// Returns `invalid_key` when `key` does not satisfy [`FieldKey`] constraints.
#[expect(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn parse_top_level_key(key: &str) -> Result<crate::key::FieldKey, ValidationError> {
    crate::key::FieldKey::new(key).map_err(|e| {
        ValidationError::builder("invalid_key")
            .at(FieldPath::root())
            .param("key", Value::String(key.to_owned()))
            .message(format!("invalid key `{key}`: {e}"))
            .build()
    })
}

#[allow(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
pub(crate) fn resolve_select_loader_key(
    fields: &[Field],
    key: &str,
) -> Result<String, ValidationError> {
    let path = FieldPath::root().join(parse_top_level_key(key)?);
    resolve_select_loader_path(fields, &path)
}

#[allow(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
pub(crate) fn resolve_select_loader_path(
    fields: &[Field],
    path: &FieldPath,
) -> Result<String, ValidationError> {
    let field = find_field_by_schema_path(fields, path)?;
    let Field::Select(select) = field else {
        return Err(loader_type_mismatch(path, "select", field.type_name()));
    };
    loader_key_or_error(select.loader.as_deref(), path)
}

#[allow(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
pub(crate) fn resolve_dynamic_loader_key(
    fields: &[Field],
    key: &str,
) -> Result<String, ValidationError> {
    let path = FieldPath::root().join(parse_top_level_key(key)?);
    resolve_dynamic_loader_path(fields, &path)
}

#[allow(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
pub(crate) fn resolve_dynamic_loader_path(
    fields: &[Field],
    path: &FieldPath,
) -> Result<String, ValidationError> {
    let field = find_field_by_schema_path(fields, path)?;
    let Field::Dynamic(dynamic) = field else {
        return Err(loader_type_mismatch(path, "dynamic", field.type_name()));
    };
    loader_key_or_error(dynamic.loader.as_deref(), path)
}

#[allow(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn loader_key_or_error(loader: Option<&str>, path: &FieldPath) -> Result<String, ValidationError> {
    loader
        .filter(|loader| !loader.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ValidationError::builder("loader.missing_config")
                .at(path.clone())
                .param("key", Value::String(path.to_string()))
                .message(format!("field `{path}` has no loader configured"))
                .build()
        })
}

fn loader_type_mismatch(path: &FieldPath, expected: &str, actual: &str) -> ValidationError {
    ValidationError::builder("field.type_mismatch")
        .at(path.clone())
        .param("key", Value::String(path.to_string()))
        .param("expected", Value::String(expected.to_owned()))
        .param("actual", Value::String(actual.to_owned()))
        .message(format!(
            "field `{path}` is not a {expected} field (got {actual})"
        ))
        .build()
}

#[allow(
    clippy::result_large_err,
    reason = "ValidationError is intentionally large; callers are on the validation path"
)]
fn find_field_by_schema_path<'a>(
    fields: &'a [Field],
    path: &FieldPath,
) -> Result<&'a Field, ValidationError> {
    let mut segments = path.segments().iter();
    let Some(PathSegment::Key(first_key)) = segments.next() else {
        return Err(ValidationError::builder("field.not_found")
            .at(path.clone())
            .param("key", Value::String(path.to_string()))
            .message(format!("field `{path}` not found in schema"))
            .build());
    };

    let mut current_path = FieldPath::root().join(first_key.clone());
    let mut current = fields
        .iter()
        .find(|field| field.key() == first_key)
        .ok_or_else(|| {
            ValidationError::builder("field.not_found")
                .at(current_path.clone())
                .param("key", Value::String(current_path.to_string()))
                .message(format!("field `{current_path}` not found in schema"))
                .build()
        })?;

    for segment in segments {
        match segment {
            PathSegment::Key(key) => match current {
                Field::Object(object) => {
                    current_path = current_path.join(key.clone());
                    current = object
                        .fields
                        .iter()
                        .find(|field| field.key() == key)
                        .ok_or_else(|| {
                            ValidationError::builder("field.not_found")
                                .at(current_path.clone())
                                .param("key", Value::String(current_path.to_string()))
                                .message(format!("field `{current_path}` not found in schema"))
                                .build()
                        })?;
                },
                Field::List(list) => {
                    let Some(item) = list.item.as_deref() else {
                        current_path = current_path.join(key.clone());
                        return Err(ValidationError::builder("field.not_found")
                            .at(current_path.clone())
                            .param("key", Value::String(current_path.to_string()))
                            .message(format!("field `{current_path}` not found in schema"))
                            .build());
                    };
                    if let Field::Object(object) = item {
                        current_path = current_path.join(key.clone());
                        current = object
                            .fields
                            .iter()
                            .find(|field| field.key() == key)
                            .ok_or_else(|| {
                                ValidationError::builder("field.not_found")
                                    .at(current_path.clone())
                                    .param("key", Value::String(current_path.to_string()))
                                    .message(format!("field `{current_path}` not found in schema"))
                                    .build()
                            })?;
                    } else {
                        current_path = current_path.join(key.clone());
                        return Err(ValidationError::builder("field.not_found")
                            .at(current_path.clone())
                            .param("key", Value::String(current_path.to_string()))
                            .message(format!("field `{current_path}` not found in schema"))
                            .build());
                    }
                },
                Field::Mode(mode) => {
                    current_path = current_path.join(key.clone());
                    current = mode
                        .variants
                        .iter()
                        .find(|variant| variant.key == key.as_str())
                        .map(|variant| variant.field.as_ref())
                        .ok_or_else(|| {
                            ValidationError::builder("field.not_found")
                                .at(current_path.clone())
                                .param("key", Value::String(current_path.to_string()))
                                .message(format!("field `{current_path}` not found in schema"))
                                .build()
                        })?;
                },
                _ => {
                    current_path = current_path.join(key.clone());
                    return Err(ValidationError::builder("field.not_found")
                        .at(current_path.clone())
                        .param("key", Value::String(current_path.to_string()))
                        .message(format!("field `{current_path}` not found in schema"))
                        .build());
                },
            },
            PathSegment::Index(index) => {
                current_path = current_path.join(*index);
                match current {
                    Field::List(list) => {
                        current = list.item.as_deref().ok_or_else(|| {
                            ValidationError::builder("field.not_found")
                                .at(current_path.clone())
                                .param("key", Value::String(current_path.to_string()))
                                .message(format!("field `{current_path}` not found in schema"))
                                .build()
                        })?;
                    },
                    _ => {
                        return Err(ValidationError::builder("field.not_found")
                            .at(current_path.clone())
                            .param("key", Value::String(current_path.to_string()))
                            .message(format!("field `{current_path}` not found in schema"))
                            .build());
                    },
                }
            },
        }
    }

    Ok(current)
}

// ── SchemaBuilder ─────────────────────────────────────────────────────────────

/// Mutable builder state. Consumed by `build()`.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    fields: Vec<Field>,
    root_rules: Vec<nebula_validator::Rule>,
}

impl SchemaBuilder {
    /// Append a field to the builder.
    #[expect(
        clippy::should_implement_trait,
        reason = "builder API mirrors add-style schema DSL"
    )]
    #[must_use]
    pub fn add(mut self, field: impl Into<Field>) -> Self {
        self.fields.push(field.into());
        self
    }

    /// Attach a schema-level rule evaluated against the full submitted value
    /// object after per-field validation succeeds.
    ///
    /// Rules are executed via [`nebula_validator::validate_rules_with_ctx`] with a
    /// [`nebula_validator::PredicateContext`] built from
    /// [`FieldValues::to_json`](crate::FieldValues::to_json).
    /// [`ExecutionMode::StaticOnly`](nebula_validator::ExecutionMode::StaticOnly) is used, so
    /// **deferred** rules (including [`Rule::custom`](nebula_validator::Rule::custom))
    /// are skipped here and remain wire hooks for the workflow engine.
    #[must_use]
    pub fn root_rule(mut self, rule: nebula_validator::Rule) -> Self {
        self.root_rules.push(rule);
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
    /// use nebula_schema::{FieldCollector, Schema, StringWidget, field_key};
    /// use nebula_validator::{Predicate, Rule};
    ///
    /// let rule = Rule::predicate(Predicate::eq("method", "POST").unwrap());
    /// let schema = Schema::builder()
    ///     .string(field_key!("method"), |s| s.required())
    ///     .group("body_section", |g| {
    ///         g.visible_when(rule)
    ///             .string(field_key!("body"), |s| s.widget(StringWidget::Multiline))
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
    /// Build a validated runtime schema.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationReport`] when structural linting or index-limit
    /// checks fail.
    pub fn build(self) -> Result<ValidSchema, ValidationReport> {
        let mut fields = self.fields;
        let mut report = ValidationReport::new();

        crate::lint::lint_tree(&fields, &FieldPath::root(), &mut report);
        validate_index_limits(&fields, &FieldPath::root(), 0, &mut report);

        if report.has_errors() {
            return Err(report);
        }

        normalize_depends_on_lists(&mut fields);

        // Build the flat path index for O(1) path lookup.
        let mut index: IndexMap<FieldPath, FieldHandle> = IndexMap::new();
        let mut flags = SchemaFlags::default();
        build_index(
            &fields,
            &FieldPath::root(),
            &SmallVec::new(),
            0,
            &mut index,
            &mut flags,
        );

        Ok(ValidSchema::from_inner(ValidSchemaInner {
            fields,
            index,
            flags,
            root_rules: self.root_rules,
        }))
    }
}

impl crate::builder::FieldCollector for SchemaBuilder {
    fn push_field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }
}

fn normalize_depends_on_lists(fields: &mut [Field]) {
    for field in fields {
        normalize_field_for_runtime(field);
    }
}

fn normalize_field_for_runtime(field: &mut Field) {
    match field {
        Field::String(string) => {
            dedupe_rules_and_transformers(&mut string.rules, &mut string.transformers);
        },
        Field::Secret(secret) => {
            dedupe_rules_and_transformers(&mut secret.rules, &mut secret.transformers);
        },
        Field::Number(number) => {
            dedupe_rules_and_transformers(&mut number.rules, &mut number.transformers);
        },
        Field::Boolean(boolean) => {
            dedupe_rules_and_transformers(&mut boolean.rules, &mut boolean.transformers);
        },
        Field::Select(select) => {
            dedupe_rules_and_transformers(&mut select.rules, &mut select.transformers);
            dedupe_depends_on_paths(&mut select.depends_on);
        },
        Field::Object(object) => {
            dedupe_rules_and_transformers(&mut object.rules, &mut object.transformers);
            normalize_depends_on_lists(&mut object.fields);
        },
        Field::List(list) => {
            dedupe_rules_and_transformers(&mut list.rules, &mut list.transformers);
            if let Some(item) = list.item.as_deref_mut() {
                normalize_field_for_runtime(item);
            }
        },
        Field::Mode(mode) => {
            dedupe_rules_and_transformers(&mut mode.rules, &mut mode.transformers);
            for variant in &mut mode.variants {
                normalize_field_for_runtime(variant.field.as_mut());
            }
        },
        Field::Code(code) => dedupe_rules_and_transformers(&mut code.rules, &mut code.transformers),
        Field::File(file) => dedupe_rules_and_transformers(&mut file.rules, &mut file.transformers),
        Field::Computed(computed) => {
            dedupe_rules_and_transformers(&mut computed.rules, &mut computed.transformers);
        },
        Field::Dynamic(dynamic) => {
            dedupe_rules_and_transformers(&mut dynamic.rules, &mut dynamic.transformers);
            dedupe_depends_on_paths(&mut dynamic.depends_on);
        },
        Field::Notice(notice) => {
            dedupe_rules_and_transformers(&mut notice.rules, &mut notice.transformers);
        },
    }
}

fn dedupe_depends_on_paths(depends_on: &mut Vec<FieldPath>) {
    let mut seen = HashSet::new();
    depends_on.retain(|path| seen.insert(path.to_string()));
}

fn dedupe_rules_and_transformers(
    rules: &mut Vec<nebula_validator::Rule>,
    transformers: &mut Vec<crate::Transformer>,
) {
    dedupe_stable_eq(rules);
    dedupe_stable_eq(transformers);
}

fn dedupe_stable_eq<T: PartialEq>(items: &mut Vec<T>) {
    let mut unique = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    *items = unique;
}

// ── Index builder ─────────────────────────────────────────────────────────────

fn build_index(
    fields: &[Field],
    prefix: &FieldPath,
    parent_cursor: &SmallVec<[u16; 4]>,
    depth: u8,
    index: &mut IndexMap<FieldPath, FieldHandle>,
    flags: &mut SchemaFlags,
) {
    use crate::mode::ExpressionMode;

    for (i, f) in fields.iter().enumerate() {
        let mut cursor = parent_cursor.clone();
        let Ok(step) = u16::try_from(i) else {
            continue;
        };
        cursor.push(step);
        let path = prefix.clone().join(f.key().clone());
        let Some(level) = depth.checked_add(1) else {
            continue;
        };
        flags.max_depth = flags.max_depth.max(level);

        // Track expression usage.
        if !matches!(f.expression(), ExpressionMode::Forbidden) {
            flags.uses_expressions = true;
        }

        // Track async loader usage.
        let has_loader = match f {
            Field::Select(s) => s
                .loader
                .as_ref()
                .is_some_and(|loader| !loader.trim().is_empty()),
            Field::Dynamic(d) => d
                .loader
                .as_ref()
                .is_some_and(|loader| !loader.trim().is_empty()),
            _ => false,
        };
        if has_loader {
            flags.has_async_loaders = true;
        }

        index.insert(
            path.clone(),
            FieldHandle {
                cursor: cursor.clone(),
                depth: level,
            },
        );

        // Recurse for container types.
        match f {
            Field::Object(obj) => {
                build_index(&obj.fields, &path, &cursor, level, index, flags);
            },
            Field::List(list) => {
                if let Some(Field::Object(o)) = list.item.as_deref() {
                    // List items are anonymous (indexed at runtime), so we do
                    // not create a dedicated `FieldPath` entry for the item
                    // itself. We only recurse when the item is an object to
                    // index its named children under the list field path.
                    build_index(&o.fields, &path, &cursor, level, index, flags);
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
        let Ok(step) = u16::try_from(vi) else {
            continue;
        };
        v_cursor.push(step);
        let variant_path = path.clone().join(vk);
        let Some(variant_depth) = depth.checked_add(2) else {
            continue;
        };
        index.insert(
            variant_path.clone(),
            FieldHandle {
                cursor: v_cursor.clone(),
                depth: variant_depth,
            },
        );
        if let Field::Object(o) = variant.field.as_ref() {
            build_index(
                &o.fields,
                &variant_path,
                &v_cursor,
                variant_depth,
                index,
                flags,
            );
        }
    }
}

fn validate_index_limits(
    fields: &[Field],
    path: &FieldPath,
    depth: u8,
    report: &mut ValidationReport,
) {
    let max_indexable_siblings = usize::from(u16::MAX) + 1;
    if fields.len() > max_indexable_siblings {
        report.push(
            ValidationError::builder("schema.index_overflow")
                .at(path.clone())
                .param("limit", Value::from(max_indexable_siblings))
                .param("actual", Value::from(fields.len()))
                .message(format!(
                    "too many sibling fields at `{path}`: {} > {}",
                    fields.len(),
                    max_indexable_siblings
                ))
                .build(),
        );
    }

    for field in fields {
        let child_path = path.clone().join(field.key().clone());
        let Some(next_depth) = depth.checked_add(1) else {
            report.push(
                ValidationError::builder("schema.depth_limit")
                    .at(child_path)
                    .param("limit", Value::from(u8::MAX))
                    .message("schema nesting depth exceeds supported index range")
                    .build(),
            );
            continue;
        };

        match field {
            Field::Object(object) => {
                validate_index_limits(&object.fields, &child_path, next_depth, report);
            },
            Field::List(list) => {
                if let Some(Field::Object(object)) = list.item.as_deref() {
                    validate_index_limits(&object.fields, &child_path, next_depth, report);
                }
            },
            Field::Mode(mode) => {
                if mode.variants.len() > max_indexable_siblings {
                    report.push(
                        ValidationError::builder("schema.index_overflow")
                            .at(child_path.clone())
                            .param("limit", Value::from(max_indexable_siblings))
                            .param("actual", Value::from(mode.variants.len()))
                            .message(format!(
                                "too many mode variants at `{child_path}`: {} > {}",
                                mode.variants.len(),
                                max_indexable_siblings
                            ))
                            .build(),
                    );
                }
                let Some(variant_depth) = depth.checked_add(2) else {
                    report.push(
                        ValidationError::builder("schema.depth_limit")
                            .at(child_path)
                            .param("limit", Value::from(u8::MAX))
                            .message("schema mode variant depth exceeds supported index range")
                            .build(),
                    );
                    continue;
                };
                for variant in &mode.variants {
                    let variant_path = crate::key::FieldKey::new(variant.key.as_str()).map_or_else(
                        |_| child_path.clone(),
                        |variant_key| child_path.clone().join(variant_key),
                    );
                    if let Field::Object(object) = variant.field.as_ref() {
                        validate_index_limits(&object.fields, &variant_path, variant_depth, report);
                    }
                }
            },
            _ => {},
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, FieldKey, FieldPath};

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
    fn loader_key_rejects_invalid_field_key_for_select() {
        let schema = Schema::builder()
            .add(Field::select(fk("select_field")))
            .build()
            .expect("valid schema");
        let err = resolve_select_loader_key(schema.fields(), "bad-key").unwrap_err();
        assert_eq!(err.code, "invalid_key");
    }

    #[test]
    fn loader_key_rejects_invalid_field_key_for_dynamic() {
        let schema = Schema::builder()
            .add(Field::dynamic(fk("dynamic_field")))
            .build()
            .expect("valid schema");
        let err = resolve_dynamic_loader_key(schema.fields(), "bad-key").unwrap_err();
        assert_eq!(err.code, "invalid_key");
    }

    #[test]
    fn build_rejects_schema_depth_beyond_index_limits() {
        fn nested_object(depth: usize) -> Field {
            let mut current: Field = Field::string(fk("leaf")).into();
            for i in (0..depth).rev() {
                let key = FieldKey::new(format!("n{i}")).expect("generated key");
                current = Field::object(key).add(current).into();
            }
            current
        }

        let result = Schema::builder().add(nested_object(260)).build();
        let report = result.expect_err("deep schema should be rejected");
        assert!(report.errors().any(|e| e.code == "schema.depth_limit"));
    }

    #[test]
    fn build_deduplicates_select_depends_on_for_runtime_schema() {
        let dep = FieldPath::parse("team_id").unwrap();
        let schema = Schema::builder()
            .add(Field::string(fk("team_id")))
            .add(
                Field::select(fk("workspace"))
                    .dynamic()
                    .loader("workspace_loader")
                    .depends_on(dep.clone())
                    .depends_on(dep),
            )
            .build()
            .expect("schema should build");

        let field = schema.find(&fk("workspace")).expect("field must exist");
        let Field::Select(select) = field else {
            panic!("expected select field");
        };
        assert_eq!(select.depends_on.len(), 1);
    }

    #[test]
    fn build_deduplicates_nested_dynamic_depends_on_for_runtime_schema() {
        let dep = FieldPath::parse("team_id").unwrap();
        let schema = Schema::builder()
            .add(Field::string(fk("team_id")))
            .add(
                Field::object(fk("container")).add(
                    Field::dynamic(fk("resource"))
                        .loader("resource_loader")
                        .depends_on(dep.clone())
                        .depends_on(dep),
                ),
            )
            .build()
            .expect("schema should build");

        let path = FieldPath::parse("container.resource").unwrap();
        let field = schema
            .find_by_path(&path)
            .expect("nested field should be indexed");
        let Field::Dynamic(dynamic) = field else {
            panic!("expected dynamic field");
        };
        assert_eq!(dynamic.depends_on.len(), 1);
    }

    #[test]
    fn build_deduplicates_rules_and_transformers_for_runtime_schema() {
        let schema = Schema::builder()
            .add(
                Field::string(fk("name"))
                    .min_length(3)
                    .min_length(3)
                    .with_transformer(crate::Transformer::Trim)
                    .with_transformer(crate::Transformer::Trim),
            )
            .build()
            .expect("schema should build");

        let field = schema.find(&fk("name")).expect("field must exist");
        let Field::String(string) = field else {
            panic!("expected string field");
        };
        assert_eq!(string.rules.len(), 1);
        assert_eq!(string.transformers.len(), 1);
    }

    #[test]
    fn build_deduplicates_nested_rules_and_transformers_for_runtime_schema() {
        let schema = Schema::builder()
            .add(
                Field::object(fk("container")).add(
                    Field::number(fk("count"))
                        .min(1)
                        .min(1)
                        .with_transformer(crate::Transformer::Trim)
                        .with_transformer(crate::Transformer::Trim),
                ),
            )
            .build()
            .expect("schema should build");

        let path = FieldPath::parse("container.count").unwrap();
        let field = schema
            .find_by_path(&path)
            .expect("nested field should be indexed");
        let Field::Number(number) = field else {
            panic!("expected number field");
        };
        assert_eq!(number.rules.len(), 1);
        assert_eq!(number.transformers.len(), 1);
    }
}
