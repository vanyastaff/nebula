use std::collections::HashMap;

use nebula_validator::{ExecutionMode, validate_rules};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    Field, FieldValues, LintReport, LoaderContext, LoaderRegistry, LoaderResult, RequiredMode,
    SchemaError, SelectOption, VisibilityMode, lint_schema,
    report::{ValidationIssue, ValidationReport},
};

/// Top-level schema definition for action/resource inputs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Ordered field list.
    fields: Vec<Field>,
}

const MAX_NESTED_DEPTH: u8 = 16;

impl Schema {
    /// Create an empty schema.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add field and return updated schema.
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

    /// Find field by key.
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
    pub fn validate(&self, values: &FieldValues, mode: ExecutionMode) -> ValidationReport {
        let mut report = ValidationReport::new();
        let context = values.as_map();

        for field in &self.fields {
            let key = field.key().as_str();
            self.validate_single_field(field, context, values.get(key), key, mode, &mut report);
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
        report: &mut ValidationReport,
    ) {
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
        report: &mut ValidationReport,
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
            Field::String(_)
            | Field::Secret(_)
            | Field::Code(_)
            | Field::Date(_)
            | Field::DateTime(_)
            | Field::Time(_)
            | Field::Color(_)
            | Field::Hidden(_) => {
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
        if depth >= MAX_NESTED_DEPTH {
            return;
        }

        if matches!(
            field,
            Field::Computed(_) | Field::Notice(_) | Field::Hidden(_)
        ) {
            return;
        }

        if !values.contains(path) {
            if let Some(default) = field.default() {
                values.set(path.to_owned(), default.clone());
            } else if let Field::Mode(mode) = field
                && let Some(default_variant) = mode.default_variant.as_deref()
            {
                values.set(
                    path.to_owned(),
                    serde_json::json!({ "mode": default_variant }),
                );
            } else {
                return;
            }
        }

        let Some(current) = values.get(path).cloned() else {
            return;
        };

        match field {
            Field::Object(object_field) => {
                let Some(mut object) = current.as_object().cloned() else {
                    return;
                };
                self.normalize_object_children(&object_field.fields, &mut object, depth + 1);
                values.set(path.to_owned(), Value::Object(object));
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
                values.set(path.to_owned(), Value::Array(normalized));
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
                    values.set(path.to_owned(), Value::Object(object));
                    return;
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

                values.set(path.to_owned(), Value::Object(object));
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
        match field {
            Field::String(_) => "string",
            Field::Secret(_) => "secret",
            Field::Number(_) => "number",
            Field::Boolean(_) => "boolean",
            Field::Select(_) => "select",
            Field::Object(_) => "object",
            Field::List(_) => "list",
            Field::Mode(_) => "mode",
            Field::Code(_) => "code",
            Field::Date(_) => "date",
            Field::DateTime(_) => "datetime",
            Field::Time(_) => "time",
            Field::Color(_) => "color",
            Field::File(_) => "file",
            Field::Hidden(_) => "hidden",
            Field::Computed(_) => "computed",
            Field::Dynamic(_) => "dynamic",
            Field::Notice(_) => "notice",
        }
    }
}
