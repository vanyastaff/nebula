use std::collections::HashMap;

use nebula_validator::{ExecutionMode, validate_rules};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Field, FieldValues, RequiredMode, ValidationIssue, ValidationReport, VisibilityMode};

/// Top-level schema definition for action/resource inputs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Ordered field list.
    fields: Vec<Field>,
}

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
            let key = field.key().as_str();
            if !normalized.contains(key)
                && let Some(default) = field.default()
            {
                normalized.set(key.to_owned(), default.clone());
            }
        }

        normalized
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

        if let Err(errors) = validate_rules(value, field.rules(), mode) {
            for error in errors.errors() {
                report.push_error(ValidationIssue::new(
                    path,
                    error.code.to_string(),
                    error.message.to_string(),
                ));
            }
        }

        self.validate_field_type(field, value, path, mode, report);
    }

    fn validate_field_type(
        &self,
        field: &Field,
        value: &Value,
        path: &str,
        mode: ExecutionMode,
        report: &mut ValidationReport,
    ) {
        match field {
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
            _ => {},
        }
    }

    fn object_to_context(object: &serde_json::Map<String, Value>) -> HashMap<String, Value> {
        object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}
