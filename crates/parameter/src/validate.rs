//! Static validation engine for parameter schemas.
//!
//! This module provides pure functions that validate a slice of [`Field`]s
//! against a [`FieldValues`] map. No expression context is required —
//! deferred rules ([`Rule::is_deferred`]) are skipped and left for runtime.

use nebula_validator::foundation::{Validate, ValidationError};
use nebula_validator::validators::{
    max as validator_max, max_size, min as validator_min, min_size,
};

use crate::error::ParameterError;
use crate::field::Field;
use crate::option::OptionSource;
use crate::profile::ValidationProfile;
use crate::report::ValidationReport;
use crate::rules::Rule;
use crate::values::FieldValues;

/// Validates `fields` against `values` using strict defaults.
///
/// Equivalent to calling [`validate_with_profile`] with [`ValidationProfile::Strict`].
///
/// # Errors
///
/// Returns a non-empty [`Vec`] of [`ParameterError`] when any field fails.
pub fn validate_fields(fields: &[Field], values: &FieldValues) -> Result<(), Vec<ParameterError>> {
    let report = validate_with_profile(fields, values, ValidationProfile::Strict);
    if report.errors.is_empty() {
        Ok(())
    } else {
        Err(report.errors)
    }
}

/// Validates `fields` against `values` under the given [`ValidationProfile`].
///
/// Returns a [`ValidationReport`] that separates hard errors from warnings.
#[must_use]
pub fn validate_with_profile(
    fields: &[Field],
    values: &FieldValues,
    profile: ValidationProfile,
) -> ValidationReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for field in fields {
        let value = values.get(&field.meta().id);
        validate_field(
            field,
            value,
            values,
            &field.meta().id,
            profile,
            &mut errors,
            &mut warnings,
        );
    }

    emit_unknown_fields(fields, values, profile, &mut errors, &mut warnings);

    ValidationReport { errors, warnings }
}

fn emit_unknown_fields(
    fields: &[Field],
    values: &FieldValues,
    profile: ValidationProfile,
    errors: &mut Vec<ParameterError>,
    warnings: &mut Vec<ParameterError>,
) {
    for key in values.keys() {
        if !fields.iter().any(|f| f.meta().id == key) {
            emit_unknown_field(key, profile, errors, warnings);
        }
    }
}

fn emit_unknown_field(
    path: &str,
    profile: ValidationProfile,
    errors: &mut Vec<ParameterError>,
    warnings: &mut Vec<ParameterError>,
) {
    let err = ParameterError::UnknownField {
        key: path.to_owned(),
    };
    match profile {
        ValidationProfile::Strict => errors.push(err),
        ValidationProfile::Warn => warnings.push(err),
        ValidationProfile::Permissive => {}
    }
}

fn validate_field(
    field: &Field,
    value: Option<&serde_json::Value>,
    root_values: &FieldValues,
    path: &str,
    profile: ValidationProfile,
    errors: &mut Vec<ParameterError>,
    warnings: &mut Vec<ParameterError>,
) {
    let meta = field.meta();
    let hidden = meta
        .visible_when
        .as_ref()
        .is_some_and(|cond| !cond.evaluate(root_values.as_map()));

    // Hidden fields are skipped unless they already have an explicit value.
    if hidden && value.is_none() {
        return;
    }

    let required_now = meta.required
        || meta
            .required_when
            .as_ref()
            .is_some_and(|cond| cond.evaluate(root_values.as_map()));

    if required_now && value.is_none_or(serde_json::Value::is_null) {
        errors.push(ParameterError::MissingValue {
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

    validate_field_value(field, value, root_values, path, profile, errors, warnings);
}

fn validate_field_value(
    field: &Field,
    value: &serde_json::Value,
    root_values: &FieldValues,
    path: &str,
    profile: ValidationProfile,
    errors: &mut Vec<ParameterError>,
    warnings: &mut Vec<ParameterError>,
) {
    let meta = field.meta();

    apply_rules(path, value, &meta.rules, errors);

    match field {
        Field::Number {
            integer, min, max, ..
        } => {
            let Some(current) = value.as_f64() else {
                errors.push(ParameterError::InvalidType {
                    key: path.to_owned(),
                    expected_type: "number".to_owned(),
                    actual_details: format!("{value:?}"),
                });
                return;
            };

            if *integer && current.fract() != 0.0 {
                errors.push(ParameterError::InvalidType {
                    key: path.to_owned(),
                    expected_type: "integer".to_owned(),
                    actual_details: format!("{value:?}"),
                });
            }

            if let Some(min) = min.as_ref().and_then(serde_json::Number::as_f64)
                && let Err(err) = validator_min(min).validate(&current)
            {
                errors.push(make_validation_issue(
                    path,
                    Some(err),
                    None,
                    format!("must be >= {min}"),
                ));
            }

            if let Some(max) = max.as_ref().and_then(serde_json::Number::as_f64)
                && let Err(err) = validator_max(max).validate(&current)
            {
                errors.push(make_validation_issue(
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
            if *allow_custom {
                return;
            }
            let OptionSource::Static { options } = source else {
                return;
            };
            if *multiple {
                let Some(items) = value.as_array() else {
                    errors.push(ParameterError::InvalidType {
                        key: path.to_owned(),
                        expected_type: "array".to_owned(),
                        actual_details: format!("{value:?}"),
                    });
                    return;
                };
                for (index, item) in items.iter().enumerate() {
                    if !options.iter().any(|opt| opt.value == *item) {
                        errors.push(ParameterError::InvalidValue {
                            key: format!("{path}.{index}"),
                            reason: "value is not part of static options".to_owned(),
                        });
                    }
                }
            } else if !options.iter().any(|opt| opt.value == *value) {
                errors.push(ParameterError::InvalidValue {
                    key: path.to_owned(),
                    reason: "value is not part of static options".to_owned(),
                });
            }
        }
        Field::Object { fields, .. } => {
            let Some(object) = value.as_object() else {
                errors.push(ParameterError::InvalidType {
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
                    profile,
                    errors,
                    warnings,
                );
            }

            for nested_key in object.keys() {
                if !fields.iter().any(|nested| nested.meta().id == *nested_key) {
                    emit_unknown_field(&format!("{path}.{nested_key}"), profile, errors, warnings);
                }
            }
        }
        Field::List {
            item,
            min_items,
            max_items,
            ..
        } => {
            let Some(items) = value.as_array() else {
                errors.push(ParameterError::InvalidType {
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
                errors.push(make_validation_issue(
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
                errors.push(make_validation_issue(
                    path,
                    Some(err),
                    None,
                    format!("must contain at most {max_items} items"),
                ));
            }

            for (index, item_value) in items.iter().enumerate() {
                let item_path = format!("{path}.{index}");
                validate_field_value(
                    item,
                    item_value,
                    root_values,
                    &item_path,
                    profile,
                    errors,
                    warnings,
                );
            }
        }
        Field::Mode {
            variants,
            default_variant,
            ..
        } => {
            let Some(object) = value.as_object() else {
                errors.push(ParameterError::InvalidType {
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
                errors.push(ParameterError::MissingValue {
                    key: format!("{path}.mode"),
                });
                return;
            };

            let Some(variant) = variants.iter().find(|variant| variant.key == mode_key) else {
                errors.push(ParameterError::InvalidValue {
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
                profile,
                errors,
                warnings,
            );

            for key in object.keys() {
                if key != "mode" && key != "value" {
                    emit_unknown_field(&format!("{path}.{key}"), profile, errors, warnings);
                }
            }
        }
        _ => {}
    }
}

fn apply_rules(
    path: &str,
    value: &serde_json::Value,
    rules: &[Rule],
    errors: &mut Vec<ParameterError>,
) {
    for rule in rules {
        if rule.is_deferred() || rule.is_predicate() {
            continue;
        }
        if let Err(err) = rule.validate_value(value) {
            errors.push(make_validation_issue(
                path,
                Some(err),
                None,
                "validation failed".to_owned(),
            ));
        }
    }
}

fn make_validation_issue(
    path: &str,
    validation_error: Option<ValidationError>,
    message_override: Option<String>,
    default_reason: String,
) -> ParameterError {
    if let Some(error) = validation_error {
        let reason =
            message_override.unwrap_or_else(|| format_validation_reason(&error, default_reason));
        return ParameterError::ValidationIssue {
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

    ParameterError::ValidationIssue {
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
