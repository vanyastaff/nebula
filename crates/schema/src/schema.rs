use nebula_validator::{ExecutionMode, validate_rules};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Field, FieldValues, RequiredMode, ValidationIssue, ValidationReport, VisibilityMode};

/// Top-level schema definition for action/resource inputs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    /// Ordered field list.
    pub fields: Vec<Field>,
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
        self.fields.push(field.into());
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

    /// Validate runtime values against this schema.
    pub fn validate(&self, values: &FieldValues, mode: ExecutionMode) -> ValidationReport {
        let mut report = ValidationReport::new();
        let context = values.as_map();

        for field in &self.fields {
            let key = field.key().as_str();
            let raw_value = values.get(key);

            let is_visible = match field.visible() {
                VisibilityMode::Always => true,
                VisibilityMode::When(rule) => rule.evaluate(context),
            };

            if !is_visible && raw_value.is_none() {
                continue;
            }

            let is_required = match field.required() {
                RequiredMode::Never => false,
                RequiredMode::Always => true,
                RequiredMode::When(rule) => rule.evaluate(context),
            };

            if is_required && raw_value.is_none_or(Value::is_null) {
                report.push_error(ValidationIssue::new(
                    key,
                    "required",
                    format!("field `{key}` is required"),
                ));
                continue;
            }

            let Some(value) = raw_value else {
                continue;
            };

            if let Err(errors) = validate_rules(value, field.rules(), mode) {
                for error in errors.errors() {
                    report.push_error(ValidationIssue::new(
                        key,
                        error.code.to_string(),
                        error.message.to_string(),
                    ));
                }
            }

            self.validate_field_type(field, value, &mut report);
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

    fn validate_field_type(&self, field: &Field, value: &Value, report: &mut ValidationReport) {
        if let Field::List(list) = field {
            let Some(array) = value.as_array() else {
                report.push_error(ValidationIssue::new(
                    list.key.as_str(),
                    "type_mismatch",
                    "list field expects array value",
                ));
                return;
            };

            if let Some(min_items) = list.min_items
                && array.len() < min_items as usize
            {
                report.push_error(ValidationIssue::new(
                    list.key.as_str(),
                    "min_items",
                    format!("expected at least {min_items} items, got {}", array.len()),
                ));
            }

            if let Some(max_items) = list.max_items
                && array.len() > max_items as usize
            {
                report.push_error(ValidationIssue::new(
                    list.key.as_str(),
                    "max_items",
                    format!("expected at most {max_items} items, got {}", array.len()),
                ));
            }
        }
    }
}
