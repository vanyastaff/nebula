//! Schema container and builder.
//!
//! `SchemaBuilder::build()` runs structural lint passes and produces a
//! `ValidSchema` proof-token. The legacy `Schema` methods (`validate`,
//! `normalize`, `load_select_options`, `load_dynamic_records`) are preserved
//! here and delegated from `ValidSchema` in Task 21.

use std::collections::HashMap;

use indexmap::IndexMap;
use nebula_validator::{ExecutionMode, validate_rules};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use smallvec::SmallVec;

use crate::{
    Field, FieldValues, LintReport, LoaderContext, LoaderRegistry, LoaderResult, RequiredMode,
    SchemaError, SelectOption, VisibilityMode,
    error::ValidationReport,
    lint_schema,
    path::FieldPath,
    report::{ValidationIssue, ValidationReport as LegacyReport},
    validated::{FieldHandle, SchemaFlags, ValidSchema, ValidSchemaInner},
};

// ── Builder entry point ───────────────────────────────────────────────────────

/// Marker type — entry point for `Schema::builder()`.
///
/// The old `Schema::new() / .add() / .validate()` API is preserved for
/// backward compatibility. Prefer `Schema::builder().add(...).build()` for
/// new code.
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

    // ── Legacy API (kept to avoid breaking existing callers) ──────────────

    /// Create an empty schema (legacy entry point).
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
    pub fn lint(&self) -> LintReport {
        lint_schema(self)
    }

    /// Validate runtime values against this schema.
    pub fn validate(&self, values: &FieldValues, mode: ExecutionMode) -> LegacyReport {
        let mut report = LegacyReport::new();
        let context = values.to_context_map();

        for field in &self.fields {
            let key = field.key().as_str();
            let raw = values.get_raw_by_str(key);
            self.validate_single_field(field, &context, raw.as_ref(), key, mode, &mut report);
        }

        report
    }

    /// Normalize runtime values by backfilling missing defaults.
    pub fn normalize(&self, values: &FieldValues) -> FieldValues {
        let mut normalized = values.clone();

        for field in &self.fields {
            let key = field.key().as_str().to_owned();
            self.normalize_field(field, &key, &mut normalized, 0);
        }

        normalized
    }

    /// Resolve dynamic options for a select field through loader registry.
    pub async fn load_select_options(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, SchemaError> {
        let field = self
            .find(key)
            .ok_or_else(|| SchemaError::FieldNotFound(key.to_owned()))?;
        let Field::Select(select) = field else {
            return Err(SchemaError::InvalidFieldType {
                key: key.to_owned(),
                expected: "select",
                actual: Self::field_type_name(field),
            });
        };
        let Some(loader_key) = select.loader.as_deref() else {
            return Err(SchemaError::LoaderNotConfigured(key.to_owned()));
        };
        registry
            .load_options(loader_key, context)
            .await
            .map_err(Into::into)
    }

    /// Resolve dynamic record payloads for a dynamic field through registry.
    pub async fn load_dynamic_records(
        &self,
        key: &str,
        registry: &LoaderRegistry,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, SchemaError> {
        let field = self
            .find(key)
            .ok_or_else(|| SchemaError::FieldNotFound(key.to_owned()))?;
        let Field::Dynamic(dynamic) = field else {
            return Err(SchemaError::InvalidFieldType {
                key: key.to_owned(),
                expected: "dynamic",
                actual: Self::field_type_name(field),
            });
        };
        let Some(loader_key) = dynamic.loader.as_deref() else {
            return Err(SchemaError::LoaderNotConfigured(key.to_owned()));
        };
        registry
            .load_records(loader_key, context)
            .await
            .map_err(Into::into)
    }

    fn validate_single_field(
        &self,
        field: &Field,
        context: &HashMap<String, Value>,
        raw_value: Option<&Value>,
        path: &str,
        mode: ExecutionMode,
        report: &mut LegacyReport,
    ) {
        const MAX_NESTED_DEPTH: u8 = 16;
        if Self::depth_from_path(path) > MAX_NESTED_DEPTH {
            report.push_error(ValidationIssue::new(
                path,
                "max_depth",
                format!("field nesting depth exceeds {MAX_NESTED_DEPTH}"),
            ));
            return;
        }

        let is_visible = match field.visible() {
            VisibilityMode::Always => true,
            VisibilityMode::Never => false,
            VisibilityMode::When(rule) => rule.evaluate(context),
        };

        if !is_visible && raw_value.is_none() {
            return;
        }

        let is_required = match field.required() {
            RequiredMode::Never => false,
            RequiredMode::Always => true,
            RequiredMode::When(rule) => rule.evaluate(context),
        };

        if is_required && raw_value.is_none_or(Value::is_null) {
            report.push_error(ValidationIssue::new(
                path,
                "required",
                format!("field `{path}` is required"),
            ));
            return;
        }

        let Some(value) = raw_value else {
            return;
        };

        let transformed = Self::apply_transformers(field, value);

        if let Err(errors) = validate_rules(&transformed, field.rules(), mode) {
            for error in errors.errors() {
                report.push_error(ValidationIssue::new(
                    path,
                    error.code.to_string(),
                    error.message.to_string(),
                ));
            }
        }

        self.validate_field_type(field, &transformed, path, mode, report);
    }

    #[expect(
        clippy::excessive_nesting,
        reason = "field-type dispatch includes nested validation branches by design"
    )]
    fn validate_field_type(
        &self,
        field: &Field,
        value: &Value,
        path: &str,
        mode: ExecutionMode,
        report: &mut LegacyReport,
    ) {
        match field {
            Field::File(file) => {
                if file.multiple {
                    let Some(items) = value.as_array() else {
                        report.push_error(ValidationIssue::new(
                            path,
                            "type_mismatch",
                            "multi-file field expects array value",
                        ));
                        return;
                    };
                    if items.iter().any(|item| !item.is_string()) {
                        report.push_error(ValidationIssue::new(
                            path,
                            "type_mismatch",
                            "multi-file field expects array of string values",
                        ));
                    }
                } else if !value.is_string() {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "file field expects string value",
                    ));
                }
            },
            Field::String(_) | Field::Secret(_) | Field::Code(_) => {
                if !value.is_string() {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "field expects string value",
                    ));
                }
            },
            Field::Computed(_) | Field::Dynamic(_) | Field::Notice(_) => {},
            Field::Number(number_field) => {
                let Some(number) = value.as_f64() else {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "number field expects numeric value",
                    ));
                    return;
                };
                if number_field.integer && number.fract() != 0.0 {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "integer field expects whole number value",
                    ));
                }
            },
            Field::Boolean(_) => {
                if !value.is_boolean() {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "boolean field expects bool value",
                    ));
                }
            },
            Field::Select(select) => {
                if select.multiple {
                    let Some(values) = value.as_array() else {
                        report.push_error(ValidationIssue::new(
                            path,
                            "type_mismatch",
                            "multi-select field expects array value",
                        ));
                        return;
                    };
                    if select.allow_custom || select.options.is_empty() {
                        return;
                    }
                    for (index, option_value) in values.iter().enumerate() {
                        let is_allowed = select
                            .options
                            .iter()
                            .any(|option| option.value == *option_value);
                        if is_allowed {
                            continue;
                        }
                        report.push_error(ValidationIssue::new(
                            format!("{path}[{index}]"),
                            "invalid_option",
                            "value is not in allowed option set",
                        ));
                    }
                } else if !select.allow_custom
                    && !select.options.is_empty()
                    && !select.options.iter().any(|option| option.value == *value)
                {
                    report.push_error(ValidationIssue::new(
                        path,
                        "invalid_option",
                        "value is not in allowed option set",
                    ));
                }
            },
            Field::List(list) => {
                let Some(array) = value.as_array() else {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "list field expects array value",
                    ));
                    return;
                };

                if let Some(min_items) = list.min_items
                    && array.len() < min_items as usize
                {
                    report.push_error(ValidationIssue::new(
                        path,
                        "min_items",
                        format!("expected at least {min_items} items, got {}", array.len()),
                    ));
                }

                if let Some(max_items) = list.max_items
                    && array.len() > max_items as usize
                {
                    report.push_error(ValidationIssue::new(
                        path,
                        "max_items",
                        format!("expected at most {max_items} items, got {}", array.len()),
                    ));
                }

                if let Some(item_schema) = list.item.as_deref() {
                    for (index, item_value) in array.iter().enumerate() {
                        let item_context = match item_value.as_object() {
                            Some(object) => Self::object_to_context(object),
                            None => HashMap::new(),
                        };
                        let item_path = format!("{path}[{index}]");
                        self.validate_single_field(
                            item_schema,
                            &item_context,
                            Some(item_value),
                            &item_path,
                            mode,
                            report,
                        );
                    }
                }
            },
            Field::Object(object_field) => {
                let Some(object) = value.as_object() else {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "object field expects object value",
                    ));
                    return;
                };

                let nested_context = Self::object_to_context(object);
                for child in &object_field.fields {
                    let child_key = child.key().as_str();
                    let child_path = format!("{path}.{child_key}");
                    self.validate_single_field(
                        child,
                        &nested_context,
                        object.get(child_key),
                        &child_path,
                        mode,
                        report,
                    );
                }
            },
            Field::Mode(mode_field) => {
                let Some(object) = value.as_object() else {
                    report.push_error(ValidationIssue::new(
                        path,
                        "type_mismatch",
                        "mode field expects object value",
                    ));
                    return;
                };

                let Some(mode_key) = object
                    .get("mode")
                    .and_then(Value::as_str)
                    .or(mode_field.default_variant.as_deref())
                else {
                    report.push_error(ValidationIssue::new(
                        path,
                        "mode_required",
                        "mode object must include `mode` key or provide default_variant",
                    ));
                    return;
                };

                let Some(variant) = mode_field.variants.iter().find(|item| item.key == mode_key)
                else {
                    report.push_error(ValidationIssue::new(
                        path,
                        "invalid_mode",
                        format!("unknown mode variant `{mode_key}`"),
                    ));
                    return;
                };

                let variant_value = object.get("value");
                let variant_context = match variant_value.and_then(Value::as_object) {
                    Some(nested) => Self::object_to_context(nested),
                    None => HashMap::new(),
                };
                let variant_path = format!("{path}.value");
                self.validate_single_field(
                    &variant.field,
                    &variant_context,
                    variant_value,
                    &variant_path,
                    mode,
                    report,
                );
            },
        }
    }

    fn apply_transformers(field: &Field, value: &Value) -> Value {
        field
            .transformers()
            .iter()
            .fold(value.clone(), |current, transformer| {
                transformer.apply(&current)
            })
    }

    fn normalize_field(&self, field: &Field, path: &str, values: &mut FieldValues, depth: u8) {
        const MAX_NESTED_DEPTH: u8 = 16;
        if depth >= MAX_NESTED_DEPTH {
            return;
        }

        if matches!(field, Field::Computed(_) | Field::Notice(_)) {
            return;
        }

        if !values.contains_str(path) {
            if let Some(default) = field.default() {
                values.set_raw(path, default.clone());
            } else if let Field::Mode(mode) = field
                && let Some(default_variant) = mode.default_variant.as_deref()
            {
                values.set_raw(path, serde_json::json!({ "mode": default_variant }));
            } else {
                return;
            }
        }

        let Some(current) = values.get_raw_by_str(path) else {
            return;
        };

        match field {
            Field::Object(object_field) => {
                let Some(mut object) = current.as_object().cloned() else {
                    return;
                };
                self.normalize_object_children(&object_field.fields, &mut object, depth + 1);
                values.set_raw(path, Value::Object(object));
            },
            Field::List(list) => {
                let Some(array) = current.as_array() else {
                    return;
                };
                let Some(item_schema) = list.item.as_deref() else {
                    return;
                };
                let mut normalized = Vec::with_capacity(array.len());
                for item in array {
                    normalized.push(self.normalize_nested_value(item_schema, item, depth + 1));
                }
                values.set_raw(path, Value::Array(normalized));
            },
            Field::Mode(mode) => {
                let Some(mut object) = current.as_object().cloned() else {
                    return;
                };
                let Some(mode_key) = object
                    .get("mode")
                    .and_then(Value::as_str)
                    .or(mode.default_variant.as_deref())
                else {
                    values.set_raw(path, Value::Object(object));
                    return;
                };
                let mode_key: String = mode_key.to_owned();

                object
                    .entry("mode".to_owned())
                    .or_insert_with(|| Value::String(mode_key.clone()));

                if let Some(variant) = mode
                    .variants
                    .iter()
                    .find(|candidate| candidate.key == mode_key)
                {
                    let normalized = if let Some(value) = object.get("value") {
                        self.normalize_nested_value(&variant.field, value, depth + 1)
                    } else if let Some(default) = variant.field.default() {
                        self.normalize_nested_value(&variant.field, default, depth + 1)
                    } else {
                        self.normalize_nested_value(
                            &variant.field,
                            &Value::Object(Map::new()),
                            depth + 1,
                        )
                    };
                    object.insert("value".to_owned(), normalized);
                }

                values.set_raw(path, Value::Object(object));
            },
            _ => {},
        }
    }

    fn normalize_object_children(
        &self,
        fields: &[Field],
        object: &mut Map<String, Value>,
        depth: u8,
    ) {
        const MAX_NESTED_DEPTH: u8 = 16;
        if depth >= MAX_NESTED_DEPTH {
            return;
        }

        for child in fields {
            let key = child.key().as_str().to_owned();
            if !object.contains_key(&key)
                && let Some(default) = child.default()
            {
                object.insert(key.clone(), default.clone());
            }

            if !object.contains_key(&key)
                && let Field::Mode(mode_field) = child
                && let Some(default_variant) = mode_field.default_variant.as_deref()
            {
                object.insert(key.clone(), serde_json::json!({ "mode": default_variant }));
            }

            if let Some(value) = object.get(&key).cloned() {
                object.insert(key, self.normalize_nested_value(child, &value, depth + 1));
            }
        }
    }

    fn normalize_nested_value(&self, field: &Field, value: &Value, depth: u8) -> Value {
        const MAX_NESTED_DEPTH: u8 = 16;
        if depth >= MAX_NESTED_DEPTH {
            return value.clone();
        }

        match field {
            Field::Object(object_field) => {
                let Some(mut object) = value.as_object().cloned() else {
                    return value.clone();
                };
                self.normalize_object_children(&object_field.fields, &mut object, depth + 1);
                Value::Object(object)
            },
            Field::List(list) => {
                let Some(array) = value.as_array() else {
                    return value.clone();
                };
                let Some(item_schema) = list.item.as_deref() else {
                    return value.clone();
                };
                let normalized = array
                    .iter()
                    .map(|item| self.normalize_nested_value(item_schema, item, depth + 1))
                    .collect();
                Value::Array(normalized)
            },
            Field::Mode(mode) => {
                let Some(mut object) = value.as_object().cloned() else {
                    return value.clone();
                };
                let Some(mode_key) = object
                    .get("mode")
                    .and_then(Value::as_str)
                    .or(mode.default_variant.as_deref())
                else {
                    return Value::Object(object);
                };
                let mode_key = mode_key.to_owned();

                object
                    .entry("mode".to_owned())
                    .or_insert_with(|| Value::String(mode_key.clone()));
                if let Some(variant) = mode
                    .variants
                    .iter()
                    .find(|candidate| candidate.key == mode_key)
                {
                    let normalized = if let Some(value) = object.get("value") {
                        self.normalize_nested_value(&variant.field, value, depth + 1)
                    } else if let Some(default) = variant.field.default() {
                        self.normalize_nested_value(&variant.field, default, depth + 1)
                    } else {
                        self.normalize_nested_value(
                            &variant.field,
                            &Value::Object(Map::new()),
                            depth + 1,
                        )
                    };
                    object.insert("value".to_owned(), normalized);
                }
                Value::Object(object)
            },
            _ => value.clone(),
        }
    }

    fn depth_from_path(path: &str) -> u8 {
        let separators = path
            .chars()
            .filter(|character| *character == '.' || *character == '[')
            .count();
        separators as u8
    }

    fn object_to_context(object: &Map<String, Value>) -> HashMap<String, Value> {
        object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    fn field_type_name(field: &Field) -> &'static str {
        field.type_name()
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
