//! Canonical v2 parameter schema — RFC 0005 reference implementation.
//!
//! The crate keeps the `parameter` name as a domain boundary. The modern
//! authoring model is field-based: [`Schema`] contains [`Field`] items plus
//! optional UI-only and layout metadata.
//!
//! Wire format: [`Field`] uses `#[serde(tag = "type")]` with
//! `#[serde(flatten)] meta: FieldMetadata` so that `id`, `label`, and all shared
//! metadata appear at the same JSON level as `"type"`, matching the canonical
//! HTTP API contract defined by RFC 0005.

pub use crate::option::OptionSource;
pub use crate::option::SelectOption;

use crate::values::ParameterValues;
use nebula_validator::foundation::{Validate, ValidationError};
use nebula_validator::validators::{
    matches_regex, max as validator_max, max_length, max_size, min as validator_min, min_length,
    min_size,
};

#[path = "field.rs"]
mod field;
pub use field::Field;
#[path = "metadata.rs"]
mod metadata;
pub use metadata::FieldMetadata;

/// Complete parameter schema for v2 authoring.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Schema {
    /// Ordered field definitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    /// UI-only elements that never appear in runtime values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui: Vec<UiElement>,
    /// Optional visual grouping metadata.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}

impl Schema {
    /// Creates an empty schema.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a field to the schema.
    #[must_use]
    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    /// Appends a UI-only element to the schema.
    #[must_use]
    pub fn ui(mut self, element: UiElement) -> Self {
        self.ui.push(element);
        self
    }

    /// Appends a field group definition.
    #[must_use]
    pub fn group(mut self, group: Group) -> Self {
        self.groups.push(group);
        self
    }

    /// Returns the number of value-bearing fields in the schema.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns `true` if the schema contains no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Returns the field with the given id, if any.
    #[must_use]
    pub fn get_field(&self, id: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.meta().id == id)
    }

    /// Returns `true` if the schema contains a field with the given id.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.fields.iter().any(|f| f.meta().id == id)
    }

    /// Validate a set of values against this schema.
    ///
    /// Checks that all required fields have a non-null value.
    ///
    /// # Errors
    ///
    /// Returns a list of [`crate::error::ParameterError`] for every required
    /// field that is missing or null.
    pub fn validate(
        &self,
        values: &crate::values::ParameterValues,
    ) -> Result<(), Vec<crate::error::ParameterError>> {
        let mut errors = Vec::new();

        for field in &self.fields {
            let value = values.get(&field.meta().id);
            validate_field(field, value, values, &field.meta().id, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn validate_field(
    field: &Field,
    value: Option<&serde_json::Value>,
    root_values: &ParameterValues,
    path: &str,
    errors: &mut Vec<crate::error::ParameterError>,
) {
    let meta = field.meta();
    let hidden = meta
        .visible_when
        .as_ref()
        .is_some_and(|cond| !evaluate_condition(cond, root_values));

    // Hidden fields are skipped unless they already have an explicit value.
    if hidden && value.is_none() {
        return;
    }

    let required_now = meta.required
        || meta
            .required_when
            .as_ref()
            .is_some_and(|cond| evaluate_condition(cond, root_values));

    if required_now && value.is_none_or(serde_json::Value::is_null) {
        errors.push(crate::error::ParameterError::MissingValue {
            key: path.to_owned(),
        });
        return;
    }

    let Some(value) = value else {
        return;
    };

    if value.is_null() {
        return;
    }

    validate_field_value(field, value, root_values, path, errors);
}

fn validate_field_value(
    field: &Field,
    value: &serde_json::Value,
    root_values: &ParameterValues,
    path: &str,
    errors: &mut Vec<crate::error::ParameterError>,
) {
    let meta = field.meta();

    apply_rules(path, value, &meta.rules, errors);

    match field {
        Field::Number {
            integer, min, max, ..
        } => {
            let Some(current) = value.as_f64() else {
                errors.push(crate::error::ParameterError::InvalidType {
                    key: path.to_owned(),
                    expected_type: "number".to_owned(),
                    actual_details: format!("{value:?}"),
                });
                return;
            };

            if *integer && current.fract() != 0.0 {
                errors.push(crate::error::ParameterError::InvalidType {
                    key: path.to_owned(),
                    expected_type: "integer".to_owned(),
                    actual_details: format!("{value:?}"),
                });
            }

            if let Some(min) = min.as_ref().and_then(serde_json::Number::as_f64)
                && let Err(err) = validator_min(min).validate(&current)
            {
                errors.push(validation_issue(
                    path,
                    Some(err),
                    None,
                    format!("must be >= {min}"),
                ));
            }

            if let Some(max) = max.as_ref().and_then(serde_json::Number::as_f64)
                && let Err(err) = validator_max(max).validate(&current)
            {
                errors.push(validation_issue(
                    path,
                    Some(err),
                    None,
                    format!("must be <= {max}"),
                ));
            }
        }
        Field::Select {
            source,
            multiple,
            allow_custom,
            ..
        } => {
            if !allow_custom && let OptionSource::Static { options } = source {
                if *multiple {
                    let Some(items) = value.as_array() else {
                        errors.push(crate::error::ParameterError::InvalidType {
                            key: path.to_owned(),
                            expected_type: "array".to_owned(),
                            actual_details: format!("{value:?}"),
                        });
                        return;
                    };

                    for (index, item) in items.iter().enumerate() {
                        if !options.iter().any(|opt| opt.value == *item) {
                            errors.push(crate::error::ParameterError::InvalidValue {
                                key: format!("{path}.{index}"),
                                reason: "value is not part of static options".to_owned(),
                            });
                        }
                    }
                } else if !options.iter().any(|opt| opt.value == *value) {
                    errors.push(crate::error::ParameterError::InvalidValue {
                        key: path.to_owned(),
                        reason: "value is not part of static options".to_owned(),
                    });
                }
            }
        }
        Field::Object { fields, .. } => {
            let Some(object) = value.as_object() else {
                errors.push(crate::error::ParameterError::InvalidType {
                    key: path.to_owned(),
                    expected_type: "object".to_owned(),
                    actual_details: format!("{value:?}"),
                });
                return;
            };

            for nested in fields {
                let nested_id = &nested.meta().id;
                let nested_path = format!("{path}.{nested_id}");
                validate_field(
                    nested,
                    object.get(nested_id),
                    root_values,
                    &nested_path,
                    errors,
                );
            }
        }
        Field::List {
            item,
            min_items,
            max_items,
            ..
        } => {
            let Some(items) = value.as_array() else {
                errors.push(crate::error::ParameterError::InvalidType {
                    key: path.to_owned(),
                    expected_type: "array".to_owned(),
                    actual_details: format!("{value:?}"),
                });
                return;
            };

            if let Some(min_items) = min_items
                && let Err(err) =
                    min_size::<serde_json::Value>(*min_items as usize).validate(items.as_slice())
            {
                errors.push(validation_issue(
                    path,
                    Some(err),
                    None,
                    format!("must contain at least {min_items} items"),
                ));
            }

            if let Some(max_items) = max_items
                && let Err(err) =
                    max_size::<serde_json::Value>(*max_items as usize).validate(items.as_slice())
            {
                errors.push(validation_issue(
                    path,
                    Some(err),
                    None,
                    format!("must contain at most {max_items} items"),
                ));
            }

            for (index, item_value) in items.iter().enumerate() {
                let item_path = format!("{path}.{index}");
                validate_field_value(item, item_value, root_values, &item_path, errors);
            }
        }
        Field::Mode {
            variants,
            default_variant,
            ..
        } => {
            let Some(object) = value.as_object() else {
                errors.push(crate::error::ParameterError::InvalidType {
                    key: path.to_owned(),
                    expected_type: "object".to_owned(),
                    actual_details: format!("{value:?}"),
                });
                return;
            };

            let mode_key = object
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .or(default_variant.as_deref());

            let Some(mode_key) = mode_key else {
                errors.push(crate::error::ParameterError::MissingValue {
                    key: format!("{path}.mode"),
                });
                return;
            };

            let Some(variant) = variants.iter().find(|variant| variant.key == mode_key) else {
                errors.push(crate::error::ParameterError::InvalidValue {
                    key: format!("{path}.mode"),
                    reason: format!("unknown mode variant `{mode_key}`"),
                });
                return;
            };

            let nested_value = object.get("value");
            let nested_path = format!("{path}.value");
            validate_field(
                &variant.content,
                nested_value,
                root_values,
                &nested_path,
                errors,
            );
        }
        _ => {}
    }
}

fn apply_rules(
    path: &str,
    value: &serde_json::Value,
    rules: &[Rule],
    errors: &mut Vec<crate::error::ParameterError>,
) {
    for rule in rules {
        match rule {
            Rule::MinLength { min, message } => {
                if let Some(string) = value.as_str()
                    && let Err(err) = min_length(*min).validate(string)
                {
                    errors.push(rule_error_with_validator(
                        path,
                        message.clone(),
                        Some(err),
                        format!("must be at least {min} characters"),
                    ));
                }
            }
            Rule::MaxLength { max, message } => {
                if let Some(string) = value.as_str()
                    && let Err(err) = max_length(*max).validate(string)
                {
                    errors.push(rule_error_with_validator(
                        path,
                        message.clone(),
                        Some(err),
                        format!("must be at most {max} characters"),
                    ));
                }
            }
            Rule::Pattern { pattern, message } => {
                if let Some(string) = value.as_str() {
                    match matches_regex(pattern) {
                        Ok(validator) => {
                            if let Err(err) = validator.validate(string) {
                                errors.push(rule_error_with_validator(
                                    path,
                                    message.clone(),
                                    Some(err),
                                    "does not match required pattern".to_owned(),
                                ));
                            }
                        }
                        Err(err) => {
                            errors.push(rule_error_with_validator(
                                path,
                                None,
                                None,
                                format!("invalid regex pattern: {err}"),
                            ));
                        }
                    }
                }
            }
            Rule::Min { min, message } => {
                if let (Some(current), Some(bound)) = (value.as_f64(), min.as_f64())
                    && let Err(err) = validator_min(bound).validate(&current)
                {
                    errors.push(rule_error_with_validator(
                        path,
                        message.clone(),
                        Some(err),
                        format!("must be >= {bound}"),
                    ));
                }
            }
            Rule::Max { max, message } => {
                if let (Some(current), Some(bound)) = (value.as_f64(), max.as_f64())
                    && let Err(err) = validator_max(bound).validate(&current)
                {
                    errors.push(rule_error_with_validator(
                        path,
                        message.clone(),
                        Some(err),
                        format!("must be <= {bound}"),
                    ));
                }
            }
            Rule::OneOf { values, message } => {
                if !values.contains(value) {
                    errors.push(rule_error_with_validator(
                        path,
                        message.clone(),
                        None,
                        "must be one of the allowed values".to_owned(),
                    ));
                }
            }
            Rule::MinItems { min, message } => {
                if let Some(items) = value.as_array()
                    && let Err(err) = min_size::<serde_json::Value>(*min).validate(items.as_slice())
                {
                    errors.push(rule_error_with_validator(
                        path,
                        message.clone(),
                        Some(err),
                        format!("must contain at least {min} items"),
                    ));
                }
            }
            Rule::MaxItems { max, message } => {
                if let Some(items) = value.as_array()
                    && let Err(err) = max_size::<serde_json::Value>(*max).validate(items.as_slice())
                {
                    errors.push(rule_error_with_validator(
                        path,
                        message.clone(),
                        Some(err),
                        format!("must contain at most {max} items"),
                    ));
                }
            }
            Rule::UniqueBy { .. } | Rule::Custom { .. } => {
                // Runtime-level evaluators handle these advanced rules.
            }
        }
    }
}

fn rule_error_with_validator(
    path: &str,
    message: Option<String>,
    validation_error: Option<ValidationError>,
    default_reason: String,
) -> crate::error::ParameterError {
    validation_issue(path, validation_error, message, default_reason)
}

fn validation_issue(
    path: &str,
    validation_error: Option<ValidationError>,
    message_override: Option<String>,
    default_reason: String,
) -> crate::error::ParameterError {
    if let Some(error) = validation_error {
        let reason =
            message_override.unwrap_or_else(|| format_validation_reason(&error, default_reason));
        return crate::error::ParameterError::ValidationIssue {
            key: path.to_owned(),
            code: error.code.clone().into_owned(),
            reason,
            params: error
                .params()
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        };
    }

    crate::error::ParameterError::ValidationIssue {
        key: path.to_owned(),
        code: "custom".to_owned(),
        reason: message_override.unwrap_or(default_reason),
        params: Vec::new(),
    }
}

fn format_validation_reason(error: &ValidationError, fallback: String) -> String {
    let code = error.code.as_ref();
    let message = error.message.as_ref();
    if message.is_empty() {
        fallback
    } else {
        format!("{code}: {message}")
    }
}

/// Evaluate a declarative [`Condition`] against runtime values.
#[must_use]
pub fn evaluate_condition(condition: &Condition, values: &ParameterValues) -> bool {
    match condition {
        Condition::Eq { field, value } => values.get(field).is_some_and(|v| v == value),
        Condition::Ne { field, value } => values.get(field).is_none_or(|v| v != value),
        Condition::Gt { field, value } => cmp_number(values.get(field), value, |a, b| a > b),
        Condition::Gte { field, value } => cmp_number(values.get(field), value, |a, b| a >= b),
        Condition::Lt { field, value } => cmp_number(values.get(field), value, |a, b| a < b),
        Condition::Lte { field, value } => cmp_number(values.get(field), value, |a, b| a <= b),
        Condition::IsTrue { field } => {
            values.get(field).and_then(serde_json::Value::as_bool) == Some(true)
        }
        Condition::IsFalse { field } => {
            values.get(field).and_then(serde_json::Value::as_bool) == Some(false)
        }
        Condition::Set { field } => values.get(field).is_some_and(|v| {
            !v.is_null()
                && match v {
                    serde_json::Value::String(s) => !s.is_empty(),
                    serde_json::Value::Array(a) => !a.is_empty(),
                    _ => true,
                }
        }),
        Condition::Empty { field } => values.get(field).is_none_or(|v| {
            v.is_null()
                || match v {
                    serde_json::Value::String(s) => s.is_empty(),
                    serde_json::Value::Array(a) => a.is_empty(),
                    _ => false,
                }
        }),
        Condition::Contains { field, value } => values.get(field).is_some_and(|v| match v {
            serde_json::Value::String(s) => value.as_str().is_some_and(|needle| s.contains(needle)),
            serde_json::Value::Array(items) => items.contains(value),
            _ => false,
        }),
        Condition::Matches { field, pattern } => values
            .get(field)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|string| {
                matches_regex(pattern).is_ok_and(|validator| validator.validate(string).is_ok())
            }),
        Condition::In {
            field,
            values: candidates,
        } => values
            .get(field)
            .is_some_and(|current| candidates.contains(current)),
        Condition::All { conditions } => conditions
            .iter()
            .all(|nested| evaluate_condition(nested, values)),
        Condition::Any { conditions } => conditions
            .iter()
            .any(|nested| evaluate_condition(nested, values)),
        Condition::Not { condition } => !evaluate_condition(condition, values),
    }
}

fn cmp_number(
    value: Option<&serde_json::Value>,
    rhs: &serde_json::Number,
    op: impl Fn(f64, f64) -> bool,
) -> bool {
    let Some(lhs) = value.and_then(serde_json::Value::as_f64) else {
        return false;
    };
    let Some(rhs) = rhs.as_f64() else {
        return false;
    };
    op(lhs, rhs)
}

/// Declarative validation rule.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum Rule {
    /// String must match the regular expression.
    Pattern {
        pattern: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// String must be at least `min` characters.
    MinLength {
        min: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// String must be at most `max` characters.
    MaxLength {
        max: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Number must be ≥ `min`.
    Min {
        min: serde_json::Number,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Number must be ≤ `max`.
    Max {
        max: serde_json::Number,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Value must be one of the given options.
    OneOf {
        values: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Collection must contain at least `min` items.
    MinItems {
        min: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Collection must contain at most `max` items.
    MaxItems {
        max: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Each list item must have a unique value for the given sub-field key.
    UniqueBy {
        /// Sub-field key path within each item (e.g. `"name"`).
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    /// Custom expression-based validation.
    Custom {
        expression: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
}

/// Deterministic condition evaluated against a live value map.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Condition {
    /// `field == value`
    Eq {
        field: String,
        value: serde_json::Value,
    },
    /// `field != value`
    Ne {
        field: String,
        value: serde_json::Value,
    },
    /// `field > value`
    Gt {
        field: String,
        value: serde_json::Number,
    },
    /// `field >= value`
    Gte {
        field: String,
        value: serde_json::Number,
    },
    /// `field < value`
    Lt {
        field: String,
        value: serde_json::Number,
    },
    /// `field <= value`
    Lte {
        field: String,
        value: serde_json::Number,
    },
    /// `field == true`
    IsTrue { field: String },
    /// `field == false`
    IsFalse { field: String },
    /// Field has a non-null, non-empty value.
    Set { field: String },
    /// Field is null, absent, or empty string/array.
    Empty { field: String },
    /// String or array field contains the given value.
    Contains {
        field: String,
        value: serde_json::Value,
    },
    /// String field matches the regular expression.
    Matches { field: String, pattern: String },
    /// Field value is a member of the given set.
    In {
        field: String,
        values: Vec<serde_json::Value>,
    },
    /// All inner conditions must hold.
    All { conditions: Vec<Condition> },
    /// At least one inner condition must hold.
    Any { conditions: Vec<Condition> },
    /// Negates the inner condition.
    Not { condition: Box<Condition> },
}

/// Non-value schema element.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiElement {
    /// Informational message.
    Notice {
        severity: Severity,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visible_when: Option<Condition>,
    },
    /// Runtime-driven action button.
    Button {
        label: String,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enabled_when: Option<Condition>,
    },
}

/// UI severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational state.
    Info,
    /// Warning state.
    Warning,
    /// Error state.
    Error,
}

/// Visual field grouping.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Group {
    /// Group title.
    pub label: String,
    /// Ordered field ids in the group.
    pub fields: Vec<String>,
    /// Whether the group is initially collapsed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

/// One variant in a [`Field::Mode`] discriminated-union field.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModeVariant {
    /// Stable variant key.
    pub key: String,
    /// Display label.
    pub label: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Single content field for this variant.
    ///
    /// Use [`Field::Object`] to group multiple sub-fields inside one variant.
    pub content: Box<Field>,
}

/// Controls when the dynamic record editor is rendered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicRecordMode {
    /// Show all provider fields.
    #[default]
    All,
    /// Show only required provider fields initially.
    #[serde(alias = "always", alias = "on_load", alias = "on_expand")]
    RequiredOnly,
}

/// Policy for values returned by a provider but absent from the schema.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownFieldPolicy {
    /// Keep unknown values and surface a warning.
    #[default]
    #[serde(alias = "preserve")]
    WarnKeep,
    /// Drop unknown values from storage.
    #[serde(alias = "drop")]
    Strip,
    /// Fail validation when unknown values are present.
    Error,
}

/// Simplified field subset that [`crate::providers::DynamicRecordProvider`]s may return.
///
/// Providers must not introduce nested [`Field::Mode`] or
/// [`Field::DynamicRecord`] variants.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldSpec {
    /// Free-form text.
    Text {
        #[serde(flatten)]
        meta: FieldMetadata,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiline: bool,
    },
    /// Number.
    Number {
        #[serde(flatten)]
        meta: FieldMetadata,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        integer: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<serde_json::Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<serde_json::Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<serde_json::Number>,
    },
    /// Boolean toggle.
    Boolean {
        #[serde(flatten)]
        meta: FieldMetadata,
    },
    /// Select with static or dynamic options.
    Select {
        #[serde(flatten)]
        meta: FieldMetadata,
        #[serde(flatten)]
        source: OptionSource,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        multiple: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        allow_custom: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        searchable: bool,
    },
}

/// Backward-compatible alias for [`FieldSpec`].
pub type DynamicFieldSpec = FieldSpec;

/// Top-level predicate expression emitted by a [`Field::Predicate`] editor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PredicateExpr {
    /// A single field-operator-value assertion.
    Rule(PredicateRule),
    /// A logical group combining multiple expressions.
    Group(PredicateGroup),
}

/// Logical combinator for a [`PredicateGroup`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateCombinator {
    /// All children must pass.
    #[default]
    And,
    /// At least one child must pass.
    Or,
}

/// A logical group of predicate expressions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PredicateGroup {
    /// How child expressions are combined.
    pub combinator: PredicateCombinator,
    /// Child expressions.
    pub children: Vec<PredicateExpr>,
}

/// A single predicate assertion.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PredicateRule {
    /// Field id the assertion applies to.
    pub field: String,
    /// Comparison operator.
    pub op: PredicateOp,
    /// Operand value (absent for unary operators like `is_set`/`is_empty`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

/// Comparison operator for a [`PredicateRule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
    /// Value is in an array of comparands.
    In,
    /// Value is not in an array.
    NotIn,
    /// String or array contains the value.
    Contains,
    /// String matches a regexp.
    Matches,
    /// Field has a non-null/non-empty value.
    IsSet,
    /// Field is null or empty.
    IsEmpty,
}

fn default_true() -> bool {
    true
}

fn default_depth() -> u8 {
    3
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn file_serializes_max_size_key() {
        let field = Field::File {
            meta: FieldMetadata {
                id: "attachment".to_owned(),
                label: "Attachment".to_owned(),
                ..FieldMetadata::default()
            },
            accept: Some("application/pdf".to_owned()),
            max_size: Some(1_024),
            multiple: false,
        };

        let value = serde_json::to_value(&field).expect("field should serialize");
        assert_eq!(value.get("max_size"), Some(&json!(1_024)));
        assert!(value.get("max_size_bytes").is_none());
    }

    #[test]
    fn file_accepts_legacy_max_size_bytes_on_input() {
        let input = json!({
            "type": "file",
            "id": "attachment",
            "label": "Attachment",
            "max_size_bytes": 2048
        });

        let field: Field = serde_json::from_value(input).expect("legacy payload should parse");
        match field {
            Field::File { max_size, .. } => assert_eq!(max_size, Some(2_048)),
            other => panic!("expected file field, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_record_serializes_mode_key() {
        let field = Field::DynamicRecord {
            meta: FieldMetadata {
                id: "row_data".to_owned(),
                label: "Row Data".to_owned(),
                ..FieldMetadata::default()
            },
            provider: "sheets.columns".to_owned(),
            depends_on: vec!["sheet_id".to_owned()],
            mode: DynamicRecordMode::RequiredOnly,
            unknown_field_policy: UnknownFieldPolicy::WarnKeep,
        };

        let value = serde_json::to_value(&field).expect("field should serialize");
        assert_eq!(value.get("mode"), Some(&json!("required_only")));
        assert!(value.get("display").is_none());
    }

    #[test]
    fn validate_reports_required_when_condition_holds() {
        let schema = Schema::new().field(Field::text("token").with_label("Token").required_when(
            Condition::Eq {
                field: "auth".to_owned(),
                value: json!("bearer"),
            },
        ));

        let mut values = ParameterValues::new();
        values.set("auth", json!("bearer"));

        let result = schema.validate(&values);
        assert!(result.is_err());
    }

    #[test]
    fn validate_applies_static_select_membership() {
        let field = Field::Select {
            meta: FieldMetadata {
                id: "method".to_owned(),
                label: "Method".to_owned(),
                ..FieldMetadata::default()
            },
            source: OptionSource::Static {
                options: vec![
                    SelectOption::new(json!("GET"), "GET"),
                    SelectOption::new(json!("POST"), "POST"),
                ],
            },
            multiple: false,
            allow_custom: false,
            searchable: false,
        };
        let schema = Schema::new().field(field);

        let mut values = ParameterValues::new();
        values.set("method", json!("PATCH"));

        let result = schema.validate(&values);
        assert!(result.is_err());
    }

    #[test]
    fn validate_emits_structured_validation_issue() {
        let schema = Schema::new().field(Field::text("username").with_label("Username").with_rule(
            Rule::MinLength {
                min: 5,
                message: None,
            },
        ));

        let mut values = ParameterValues::new();
        values.set("username", json!("abc"));

        let result = schema
            .validate(&values)
            .expect_err("value must fail min_length");
        assert!(matches!(
            &result[0],
            crate::error::ParameterError::ValidationIssue {
                key,
                code,
                reason,
                ..
            } if key == "username" && code == "min_length" && !reason.is_empty()
        ));
    }
}
