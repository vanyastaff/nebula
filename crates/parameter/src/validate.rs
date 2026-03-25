//! Static validation engine for parameter schemas.
//!
//! Validates [`Parameter`](crate::parameter::Parameter) definitions against
//! [`ParameterValues`](crate::values::ParameterValues), producing structured
//! errors and warnings via [`ValidationReport`](crate::report::ValidationReport).

use std::collections::HashSet;

use serde_json::Value;

use crate::error::ParameterError;
use crate::parameter::Parameter;
use crate::parameter_type::ParameterType;
use crate::profile::ValidationProfile;
use crate::report::ValidationReport;
use crate::values::ParameterValues;

/// Maximum recursion depth for nested validation (Object → Object → ...).
const MAX_VALIDATE_DEPTH: u8 = 16;

/// Validates `parameters` against `values` using strict defaults.
///
/// # Errors
///
/// Returns a non-empty [`Vec`] of [`ParameterError`] when any parameter fails.
pub fn validate_parameters(
    parameters: &[Parameter],
    values: &ParameterValues,
) -> Result<(), Vec<ParameterError>> {
    let report = validate_with_profile(parameters, values, ValidationProfile::Strict);
    if report.errors.is_empty() {
        Ok(())
    } else {
        Err(report.errors)
    }
}

/// Validates `parameters` against `values` under the given [`ValidationProfile`].
///
/// Returns a [`ValidationReport`] that separates hard errors from warnings.
#[must_use]
pub fn validate_with_profile(
    parameters: &[Parameter],
    values: &ParameterValues,
    profile: ValidationProfile,
) -> ValidationReport {
    let mut report = ValidationReport::default();
    let values_map = values.as_map();

    for param in parameters {
        validate_parameter(param, values, values_map, "", 0, &mut report);
    }

    // Unknown field check.
    let known_ids: HashSet<&str> = parameters.iter().map(|p| p.id.as_str()).collect();
    for key in values.keys() {
        if !known_ids.contains(key) {
            let error = ParameterError::UnknownField {
                key: key.to_owned(),
            };
            match profile {
                ValidationProfile::Strict => report.errors.push(error),
                ValidationProfile::Warn => report.warnings.push(error),
                ValidationProfile::Permissive => {}
            }
        }
    }

    report
}

/// Validates a single parameter against the values map.
fn validate_parameter(
    param: &Parameter,
    values: &ParameterValues,
    values_map: &std::collections::HashMap<String, Value>,
    path_prefix: &str,
    depth: u8,
    report: &mut ValidationReport,
) {
    if depth >= MAX_VALIDATE_DEPTH {
        return;
    }

    let key = make_path(path_prefix, &param.id);

    // 1. Skip Computed and Notice — no validation needed.
    if matches!(
        param.param_type,
        ParameterType::Computed { .. } | ParameterType::Notice { .. }
    ) {
        return;
    }

    let raw_value = lookup_value(values, &param.id);

    // 2. Check visible_when — if hidden and no value, skip entirely.
    if let Some(condition) = &param.visible_when
        && !condition.evaluate(values_map)
        && raw_value.is_none()
    {
        return;
    }

    // 3. Check required.
    let is_required = param.required
        || param
            .required_when
            .as_ref()
            .is_some_and(|c| c.evaluate(values_map));

    if is_required && is_missing_or_null(raw_value) {
        report
            .errors
            .push(ParameterError::MissingValue { key: key.clone() });
        return;
    }

    // 4/5. If no value or null, skip rest.
    let Some(value) = raw_value else { return };
    if value.is_null() {
        return;
    }

    // 6. Apply rules — skip deferred and predicate rules.
    for rule in &param.rules {
        if rule.is_deferred() || rule.is_predicate() {
            continue;
        }
        if let Err(validation_error) = rule.validate_value(value) {
            report.errors.push(ParameterError::ValidationIssue {
                key: key.clone(),
                code: validation_error.code.to_string(),
                reason: validation_error.message.to_string(),
                params: validation_error
                    .params()
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            });
        }
    }

    // 7. Type-specific validation.
    validate_type(&param.param_type, value, &key, depth, report);
}

/// Type-specific validation dispatch.
fn validate_type(
    param_type: &ParameterType,
    value: &Value,
    key: &str,
    depth: u8,
    report: &mut ValidationReport,
) {
    match param_type {
        ParameterType::Number {
            integer, min, max, ..
        } => validate_number(value, *integer, min.as_ref(), max.as_ref(), key, report),

        ParameterType::Select {
            options,
            multiple,
            allow_custom,
            dynamic,
            ..
        } => validate_select(
            value,
            options,
            *multiple,
            *allow_custom,
            *dynamic,
            key,
            report,
        ),

        ParameterType::Object {
            parameters,
            display_mode,
        } => validate_object(value, parameters, display_mode, key, depth, report),

        ParameterType::List {
            item,
            min_items,
            max_items,
            ..
        } => validate_list(value, item, *min_items, *max_items, key, depth, report),

        ParameterType::Mode {
            variants,
            default_variant,
        } => validate_mode(value, variants, default_variant.as_deref(), key, depth, report),

        ParameterType::Dynamic { .. } | ParameterType::Filter { .. } => {
            // Dynamic: resolved at runtime, skip.
            // Filter: skip deep validation for now.
        }

        // All others: no type-specific validation beyond rules.
        ParameterType::String { .. }
        | ParameterType::Boolean
        | ParameterType::Code { .. }
        | ParameterType::Date
        | ParameterType::DateTime
        | ParameterType::Time
        | ParameterType::Color
        | ParameterType::File { .. }
        | ParameterType::Hidden
        | ParameterType::Computed { .. }
        | ParameterType::Notice { .. } => {}
    }
}

// ── Number ──────────────────────────────────────────────────────────────────

fn validate_number(
    value: &Value,
    integer: bool,
    min: Option<&serde_json::Number>,
    max: Option<&serde_json::Number>,
    key: &str,
    report: &mut ValidationReport,
) {
    let Some(n) = value.as_f64() else {
        report.errors.push(ParameterError::InvalidType {
            key: key.to_owned(),
            expected_type: "number".to_owned(),
            actual_details: value_type_name(value),
        });
        return;
    };

    if integer && n.fract() != 0.0 {
        report.errors.push(ParameterError::InvalidType {
            key: key.to_owned(),
            expected_type: "integer".to_owned(),
            actual_details: format!("float {n}"),
        });
        return;
    }

    if let Some(min_num) = min
        && let Some(min_f) = min_num.as_f64()
        && n < min_f
    {
        report.errors.push(ParameterError::ValidationIssue {
            key: key.to_owned(),
            code: "number_min".to_owned(),
            reason: format!("value {n} is below minimum {min_f}"),
            params: vec![("min".to_owned(), min_f.to_string())],
        });
    }

    if let Some(max_num) = max
        && let Some(max_f) = max_num.as_f64()
        && n > max_f
    {
        report.errors.push(ParameterError::ValidationIssue {
            key: key.to_owned(),
            code: "number_max".to_owned(),
            reason: format!("value {n} exceeds maximum {max_f}"),
            params: vec![("max".to_owned(), max_f.to_string())],
        });
    }
}

// ── Select ──────────────────────────────────────────────────────────────────

fn validate_select(
    value: &Value,
    options: &[crate::option::SelectOption],
    multiple: bool,
    allow_custom: bool,
    dynamic: bool,
    key: &str,
    report: &mut ValidationReport,
) {
    if allow_custom || (dynamic && options.is_empty()) {
        return;
    }

    if multiple {
        let Some(arr) = value.as_array() else {
            report.errors.push(ParameterError::InvalidType {
                key: key.to_owned(),
                expected_type: "array (multi-select)".to_owned(),
                actual_details: value_type_name(value),
            });
            return;
        };
        for item in arr {
            if !options.iter().any(|o| o.value == *item) {
                report.errors.push(ParameterError::InvalidValue {
                    key: key.to_owned(),
                    reason: format!("value {item} is not in the allowed options"),
                });
            }
        }
    } else if !options.is_empty() && !options.iter().any(|o| o.value == *value) {
        report.errors.push(ParameterError::InvalidValue {
            key: key.to_owned(),
            reason: format!("value {value} is not in the allowed options"),
        });
    }
}

// ── Object ──────────────────────────────────────────────────────────────────

fn validate_object(
    value: &Value,
    parameters: &[Parameter],
    display_mode: &crate::display_mode::DisplayMode,
    key: &str,
    depth: u8,
    report: &mut ValidationReport,
) {
    let Some(obj) = value.as_object() else {
        report.errors.push(ParameterError::InvalidType {
            key: key.to_owned(),
            expected_type: "object".to_owned(),
            actual_details: value_type_name(value),
        });
        return;
    };

    // Build a nested ParameterValues from the object for condition evaluation.
    let nested_values: ParameterValues = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let nested_map = nested_values.as_map();

    let is_pick_mode = display_mode.is_pick_mode();

    for sub_param in parameters {
        // In pick mode, skip sub-parameters whose key is absent from the object.
        if is_pick_mode && !obj.contains_key(&sub_param.id) {
            continue;
        }
        validate_parameter(sub_param, &nested_values, nested_map, key, depth + 1, report);
    }

    // Check for unknown fields within the object.
    let known_sub_ids: HashSet<&str> = parameters.iter().map(|p| p.id.as_str()).collect();
    for obj_key in obj.keys() {
        if !known_sub_ids.contains(obj_key.as_str()) {
            report.warnings.push(ParameterError::UnknownField {
                key: make_path(key, obj_key),
            });
        }
    }
}

// ── List ────────────────────────────────────────────────────────────────────

fn validate_list(
    value: &Value,
    item: &Parameter,
    min_items: Option<u32>,
    max_items: Option<u32>,
    key: &str,
    depth: u8,
    report: &mut ValidationReport,
) {
    let Some(arr) = value.as_array() else {
        report.errors.push(ParameterError::InvalidType {
            key: key.to_owned(),
            expected_type: "array".to_owned(),
            actual_details: value_type_name(value),
        });
        return;
    };

    let len = arr.len();

    if let Some(min) = min_items
        && len < min as usize
    {
        report.errors.push(ParameterError::ValidationIssue {
            key: key.to_owned(),
            code: "min_items".to_owned(),
            reason: format!("expected at least {min} items, got {len}"),
            params: vec![("min".to_owned(), min.to_string())],
        });
    }

    if let Some(max) = max_items
        && len > max as usize
    {
        report.errors.push(ParameterError::ValidationIssue {
            key: key.to_owned(),
            code: "max_items".to_owned(),
            reason: format!("expected at most {max} items, got {len}"),
            params: vec![("max".to_owned(), max.to_string())],
        });
    }

    // Recurse into each item.
    for (i, item_value) in arr.iter().enumerate() {
        let item_key = make_path(key, &i.to_string());
        // Build a single-entry ParameterValues for the item.
        let item_values: ParameterValues = vec![(item.id.clone(), item_value.clone())]
            .into_iter()
            .collect();
        let item_map = item_values.as_map();
        validate_parameter(item, &item_values, item_map, &item_key, depth + 1, report);
    }
}

// ── Mode ────────────────────────────────────────────────────────────────────

fn validate_mode(
    value: &Value,
    variants: &[Parameter],
    default_variant: Option<&str>,
    key: &str,
    depth: u8,
    report: &mut ValidationReport,
) {
    let Some(obj) = value.as_object() else {
        report.errors.push(ParameterError::InvalidType {
            key: key.to_owned(),
            expected_type: "object (mode)".to_owned(),
            actual_details: value_type_name(value),
        });
        return;
    };

    let mode_key = obj.get("mode").and_then(Value::as_str).or(default_variant);

    let Some(mode_key) = mode_key else {
        report.errors.push(ParameterError::InvalidValue {
            key: key.to_owned(),
            reason: "mode object missing \"mode\" key and no default variant".to_owned(),
        });
        return;
    };

    // Find matching variant by id.
    let variant = variants.iter().find(|v| v.id == mode_key);
    let Some(variant) = variant else {
        report.errors.push(ParameterError::InvalidValue {
            key: key.to_owned(),
            reason: format!("unknown mode variant \"{mode_key}\""),
        });
        return;
    };

    // Validate variant's content under "value" key.
    if let Some(variant_value) = obj.get("value")
        && !variant_value.is_null()
    {
        let variant_values: ParameterValues = vec![(variant.id.clone(), variant_value.clone())]
            .into_iter()
            .collect();
        let variant_map = variant_values.as_map();
        validate_parameter(variant, &variant_values, variant_map, key, depth + 1, report);
    }

    // Check for unknown keys (only "mode" and "value" allowed).
    for obj_key in obj.keys() {
        if obj_key != "mode" && obj_key != "value" {
            report.warnings.push(ParameterError::UnknownField {
                key: make_path(key, obj_key),
            });
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Builds a dot-separated path from a prefix and segment.
fn make_path(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_owned()
    } else {
        format!("{prefix}.{segment}")
    }
}

/// Looks up a value from `ParameterValues` by key.
fn lookup_value<'a>(values: &'a ParameterValues, id: &str) -> Option<&'a Value> {
    values.get(id)
}

/// Returns `true` if the value is absent or JSON null.
fn is_missing_or_null(value: Option<&Value>) -> bool {
    value.is_none_or(Value::is_null)
}

/// Returns a human-readable type name for a JSON value.
fn value_type_name(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => format!("boolean {b}"),
        Value::Number(n) => format!("number {n}"),
        Value::String(s) => {
            let truncated: String = s.chars().take(20).collect();
            if truncated.len() < s.len() {
                format!("string \"{truncated}...\"")
            } else {
                format!("string \"{s}\"")
            }
        }
        Value::Array(arr) => format!("array (length {})", arr.len()),
        Value::Object(_) => "object".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::parameter::Parameter;

    #[test]
    fn empty_params_empty_values_passes() {
        let result = validate_parameters(&[], &ParameterValues::new());
        assert!(result.is_ok());
    }

    #[test]
    fn missing_required_string_fails() {
        let params = vec![Parameter::string("name").required()];
        let values = ParameterValues::new();
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(matches!(&errors[0], ParameterError::MissingValue { key } if key == "name"));
    }

    #[test]
    fn provided_required_string_passes() {
        let params = vec![Parameter::string("name").required()];
        let mut values = ParameterValues::new();
        values.set("name", json!("Alice"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn null_value_treated_as_missing_for_required() {
        let params = vec![Parameter::string("name").required()];
        let mut values = ParameterValues::new();
        values.set("name", Value::Null);
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
    }

    #[test]
    fn optional_missing_is_ok() {
        let params = vec![Parameter::string("name")];
        let values = ParameterValues::new();
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn unknown_field_strict_is_error() {
        let params = vec![Parameter::string("name")];
        let mut values = ParameterValues::new();
        values.set("name", json!("Alice"));
        values.set("extra", json!("oops"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParameterError::UnknownField { key } if key == "extra"))
        );
    }

    #[test]
    fn unknown_field_warn_is_warning() {
        let params = vec![Parameter::string("name")];
        let mut values = ParameterValues::new();
        values.set("name", json!("Alice"));
        values.set("extra", json!("oops"));
        let report = validate_with_profile(&params, &values, ValidationProfile::Warn);
        assert!(report.is_ok());
        assert!(report.has_warnings());
    }

    #[test]
    fn unknown_field_permissive_ignored() {
        let params = vec![Parameter::string("name")];
        let mut values = ParameterValues::new();
        values.set("name", json!("Alice"));
        values.set("extra", json!("oops"));
        let report = validate_with_profile(&params, &values, ValidationProfile::Permissive);
        assert!(report.is_ok());
        assert!(!report.has_warnings());
    }

    #[test]
    fn number_rejects_non_numeric() {
        let params = vec![Parameter::number("count").required()];
        let mut values = ParameterValues::new();
        values.set("count", json!("not a number"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
        assert!(matches!(
            &result.unwrap_err()[0],
            ParameterError::InvalidType { key, expected_type, .. }
            if key == "count" && expected_type == "number"
        ));
    }

    #[test]
    fn integer_rejects_float() {
        let params = vec![Parameter::integer("count").required()];
        let mut values = ParameterValues::new();
        values.set("count", json!(3.5));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
        assert!(matches!(
            &result.unwrap_err()[0],
            ParameterError::InvalidType { key, expected_type, .. }
            if key == "count" && expected_type == "integer"
        ));
    }

    #[test]
    fn integer_accepts_whole_number() {
        let params = vec![Parameter::integer("count").required()];
        let mut values = ParameterValues::new();
        values.set("count", json!(42));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn number_min_max_validation() {
        let params = vec![Parameter::number("port").min(1.0).max(65535.0).required()];
        let mut values = ParameterValues::new();

        values.set("port", json!(0));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());

        values.set("port", json!(70000));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());

        values.set("port", json!(8080));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn select_rejects_invalid_option() {
        let params = vec![
            Parameter::select("color")
                .option(json!("red"), "Red")
                .option(json!("blue"), "Blue")
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("color", json!("green"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
    }

    #[test]
    fn select_accepts_valid_option() {
        let params = vec![
            Parameter::select("color")
                .option(json!("red"), "Red")
                .option(json!("blue"), "Blue")
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("color", json!("red"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn select_allow_custom_skips_check() {
        let params = vec![
            Parameter::select("color")
                .option(json!("red"), "Red")
                .allow_custom()
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("color", json!("chartreuse"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn multi_select_validates_each_item() {
        let params = vec![
            Parameter::select("colors")
                .option(json!("red"), "Red")
                .option(json!("blue"), "Blue")
                .multiple()
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("colors", json!(["red", "green"]));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
    }

    #[test]
    fn list_validates_min_max_items() {
        let params = vec![
            Parameter::list("tags", Parameter::string("tag"))
                .min_items(1)
                .max_items(3)
                .required(),
        ];
        let mut values = ParameterValues::new();

        values.set("tags", json!([]));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());

        values.set("tags", json!(["a", "b", "c", "d"]));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());

        values.set("tags", json!(["a", "b"]));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn list_rejects_non_array() {
        let params = vec![Parameter::list("tags", Parameter::string("tag")).required()];
        let mut values = ParameterValues::new();
        values.set("tags", json!("not an array"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
        assert!(matches!(
            &result.unwrap_err()[0],
            ParameterError::InvalidType { expected_type, .. } if expected_type == "array"
        ));
    }

    #[test]
    fn object_rejects_non_object() {
        let params = vec![
            Parameter::object("config")
                .add(Parameter::string("host"))
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("config", json!("not an object"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
    }

    #[test]
    fn object_validates_nested_required() {
        let params = vec![
            Parameter::object("config")
                .add(Parameter::string("host").required())
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("config", json!({}));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
        assert!(matches!(
            &result.unwrap_err()[0],
            ParameterError::MissingValue { key } if key == "config.host"
        ));
    }

    #[test]
    fn object_nested_passes_when_present() {
        let params = vec![
            Parameter::object("config")
                .add(Parameter::string("host").required())
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("config", json!({"host": "localhost"}));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn mode_rejects_non_object() {
        let params = vec![
            Parameter::mode("auth")
                .variant(Parameter::string("bearer"))
                .variant(Parameter::string("basic"))
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("auth", json!("not an object"));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
    }

    #[test]
    fn mode_rejects_unknown_variant() {
        let params = vec![
            Parameter::mode("auth")
                .variant(Parameter::string("bearer"))
                .variant(Parameter::string("basic"))
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("auth", json!({"mode": "oauth2"}));
        let result = validate_parameters(&params, &values);
        assert!(result.is_err());
        assert!(matches!(
            &result.unwrap_err()[0],
            ParameterError::InvalidValue { reason, .. } if reason.contains("oauth2")
        ));
    }

    #[test]
    fn mode_accepts_valid_variant() {
        let params = vec![
            Parameter::mode("auth")
                .variant(Parameter::string("bearer"))
                .variant(Parameter::string("basic"))
                .required(),
        ];
        let mut values = ParameterValues::new();
        values.set("auth", json!({"mode": "bearer", "value": "token123"}));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn computed_always_skipped() {
        let params = vec![Parameter::computed("full_name")];
        let values = ParameterValues::new();
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn notice_always_skipped() {
        let params = vec![Parameter::notice("info_banner")];
        let values = ParameterValues::new();
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }

    #[test]
    fn visible_when_hidden_and_absent_skips() {
        let params = vec![
            Parameter::string("token")
                .required()
                .visible_when(crate::conditions::Condition::eq("auth", "oauth2")),
        ];
        let mut values = ParameterValues::new();
        values.set("auth", json!("basic"));
        // token is required but hidden (visible_when false) and absent → skip
        let report = validate_with_profile(&params, &values, ValidationProfile::Permissive);
        assert!(report.is_ok());
    }

    #[test]
    fn visible_when_hidden_but_value_present_validates() {
        let params = vec![
            Parameter::string("token")
                .required()
                .visible_when(crate::conditions::Condition::eq("auth", "oauth2")),
        ];
        let mut values = ParameterValues::new();
        values.set("auth", json!("basic"));
        values.set("token", json!("some-value"));
        // token is hidden but a value IS present → still validate
        let report = validate_with_profile(&params, &values, ValidationProfile::Permissive);
        assert!(report.is_ok());
    }

    #[test]
    fn required_when_condition_true_enforces_requirement() {
        let params = vec![
            Parameter::string("token")
                .required_when(crate::conditions::Condition::eq("auth", "oauth2")),
        ];
        let mut values = ParameterValues::new();
        values.set("auth", json!("oauth2"));
        // token is required because auth == oauth2, but missing
        let report = validate_with_profile(&params, &values, ValidationProfile::Permissive);
        assert!(report.has_errors());
        assert!(matches!(
            &report.errors[0],
            ParameterError::MissingValue { key } if key == "token"
        ));
    }

    #[test]
    fn required_when_condition_false_skips_requirement() {
        let params = vec![
            Parameter::string("token")
                .required_when(crate::conditions::Condition::eq("auth", "oauth2")),
        ];
        let mut values = ParameterValues::new();
        values.set("auth", json!("basic"));
        let report = validate_with_profile(&params, &values, ValidationProfile::Permissive);
        assert!(report.is_ok());
    }

    #[test]
    fn dynamic_type_skipped() {
        let params = vec![Parameter::dynamic("custom").required()];
        let mut values = ParameterValues::new();
        values.set("custom", json!({"anything": "goes"}));
        let result = validate_parameters(&params, &values);
        assert!(result.is_ok());
    }
}
