//! Default and mode normalization helpers.
//!
//! Walks a [`Parameter`] schema and backfills missing values from `default`
//! metadata and mode `default_variant` fields. Existing user-provided values
//! are never overwritten.

use serde_json::Value;

use crate::display_mode::DisplayMode;
use crate::parameter::Parameter;
use crate::parameter_type::ParameterType;
use crate::values::ParameterValues;

/// Maximum recursion depth for nested normalization.
const MAX_NORMALIZE_DEPTH: u8 = 16;

/// Applies schema defaults to `values` for each parameter in `parameters`.
///
/// Existing user-provided values are preserved. Missing parameters are
/// materialized from `default` metadata and mode default variants.
///
/// Extra keys in `values` that are not present in the schema are preserved
/// (normalization is not validation).
///
/// # Examples
///
/// ```
/// use nebula_parameter::parameter::Parameter;
/// use nebula_parameter::values::ParameterValues;
/// use nebula_parameter::normalize::normalize_parameters;
/// use serde_json::json;
///
/// let params = vec![
///     Parameter::string("name").default(json!("anonymous")),
///     Parameter::integer("retries").default(json!(3)),
/// ];
///
/// let values = ParameterValues::new();
/// let result = normalize_parameters(&params, &values);
///
/// assert_eq!(result.get("name"), Some(&json!("anonymous")));
/// assert_eq!(result.get("retries"), Some(&json!(3)));
/// ```
#[must_use]
pub fn normalize_parameters(parameters: &[Parameter], values: &ParameterValues) -> ParameterValues {
    let mut result = values.clone();
    normalize_slice(parameters, &mut result, 0);
    result
}

/// Normalize a flat slice of parameters into the given value map.
fn normalize_slice(parameters: &[Parameter], values: &mut ParameterValues, depth: u8) {
    if depth >= MAX_NORMALIZE_DEPTH {
        return;
    }
    for param in parameters {
        normalize_one(param, values, depth);
    }
}

/// Normalize a single parameter entry.
fn normalize_one(param: &Parameter, values: &mut ParameterValues, depth: u8) {
    match &param.param_type {
        // Computed values are evaluated at runtime — skip.
        ParameterType::Computed { .. } => return,
        // Notice is display-only — no value to normalize.
        ParameterType::Notice { .. } => return,
        // Hidden parameters never get defaults backfilled.
        ParameterType::Hidden => return,
        _ => {}
    }

    let has_value = values.contains(&param.id);

    if !has_value {
        // Backfill from `param.default` if present.
        if let Some(default) = &param.default {
            values.set(&param.id, default.clone());
            // After backfilling, recurse into the newly set value for nested types.
            normalize_nested(param, values, depth);
            return;
        }

        // Mode: backfill from `default_variant` when no value exists.
        if let ParameterType::Mode {
            default_variant: Some(variant_id),
            ..
        } = &param.param_type
        {
            let mode_obj = serde_json::json!({ "mode": variant_id });
            values.set(&param.id, mode_obj);
            // Recurse into the newly created mode value.
            normalize_nested(param, values, depth);
            return;
        }

        // No value and no default — nothing to do.
        return;
    }

    // Value exists — recurse into nested types.
    normalize_nested(param, values, depth);
}

/// Handle recursion into Mode, Object, and List values.
fn normalize_nested(param: &Parameter, values: &mut ParameterValues, depth: u8) {
    match &param.param_type {
        ParameterType::Mode {
            variants,
            default_variant,
        } => {
            normalize_mode(
                &param.id,
                variants,
                default_variant.as_deref(),
                values,
                depth,
            );
        }
        ParameterType::Object {
            parameters,
            display_mode,
        } => {
            normalize_object(&param.id, parameters, *display_mode, values, depth);
        }
        ParameterType::List { item, .. } => {
            normalize_list(&param.id, item, values, depth);
        }
        _ => {}
    }
}

/// Normalize a Mode parameter's value.
///
/// Ensures the `"mode"` key exists (backfills from `default_variant`) and
/// recurses into the selected variant for nested defaults.
fn normalize_mode(
    key: &str,
    variants: &[Parameter],
    default_variant: Option<&str>,
    values: &mut ParameterValues,
    depth: u8,
) {
    let Some(raw) = values.get(key).cloned() else {
        return;
    };
    let Some(obj) = raw.as_object() else {
        return;
    };

    let mut obj = obj.clone();

    // Ensure "mode" key exists.
    if !obj.contains_key("mode") {
        if let Some(dv) = default_variant {
            obj.insert("mode".to_owned(), Value::String(dv.to_owned()));
        } else {
            // No mode key and no default variant — can't proceed.
            values.set(key, Value::Object(obj));
            return;
        }
    }

    let mode_key = match obj.get("mode").and_then(Value::as_str) {
        Some(m) => m.to_owned(),
        None => {
            values.set(key, Value::Object(obj));
            return;
        }
    };

    // Find the matching variant.
    let variant = variants.iter().find(|v| v.id == mode_key);
    if let Some(variant) = variant {
        // Hidden variant — no "value" key needed, skip recursion.
        if matches!(variant.param_type, ParameterType::Hidden) {
            values.set(key, Value::Object(obj));
            return;
        }

        match &variant.param_type {
            ParameterType::Object { parameters, .. } => {
                // Recurse into "value" as an object.
                let inner = obj
                    .get("value")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();

                let mut inner_values = object_to_parameter_values(&inner);
                normalize_slice(parameters, &mut inner_values, depth + 1);
                obj.insert(
                    "value".to_owned(),
                    parameter_values_to_object(&inner_values),
                );
            }
            _ => {
                // Scalar or other variant — backfill "value" from variant default.
                if !obj.contains_key("value")
                    && let Some(default) = &variant.default
                {
                    obj.insert("value".to_owned(), default.clone());
                }
            }
        }
    }

    values.set(key, Value::Object(obj));
}

/// Normalize an Object parameter's value.
fn normalize_object(
    key: &str,
    parameters: &[Parameter],
    display_mode: DisplayMode,
    values: &mut ParameterValues,
    depth: u8,
) {
    let Some(raw) = values.get(key).cloned() else {
        return;
    };
    let Some(obj) = raw.as_object() else {
        return;
    };

    let mut inner_values = object_to_parameter_values(obj);

    if display_mode.is_pick_mode() {
        // PickFields/Sections: only normalize keys already present.
        // For sub-params whose key is absent, set to {} if the sub-param
        // is an Object type. For keys that ARE present, recurse but don't
        // backfill absent fields.
        normalize_pick_mode(parameters, &mut inner_values, depth);
    } else {
        // Inline/Collapsed: recurse into all sub-parameters, backfill defaults.
        normalize_slice(parameters, &mut inner_values, depth + 1);
    }

    values.set(key, parameter_values_to_object(&inner_values));
}

/// Normalize sub-parameters in pick mode.
///
/// Only processes keys that are already present in the inner object.
/// Does NOT backfill defaults for absent keys (the user hasn't "picked" them yet).
fn normalize_pick_mode(parameters: &[Parameter], values: &mut ParameterValues, depth: u8) {
    for param in parameters {
        match &param.param_type {
            ParameterType::Computed { .. }
            | ParameterType::Notice { .. }
            | ParameterType::Hidden => {
                continue;
            }
            _ => {}
        }

        if !values.contains(&param.id) {
            // Absent in pick mode — skip (don't backfill defaults).
            continue;
        }

        // Key is present — recurse into nested structures.
        normalize_nested(param, values, depth + 1);
    }
}

/// Normalize a List parameter's value.
///
/// Recurses into each array item using the item template parameter.
fn normalize_list(key: &str, item_template: &Parameter, values: &mut ParameterValues, depth: u8) {
    let Some(raw) = values.get(key).cloned() else {
        return;
    };
    let Some(arr) = raw.as_array() else {
        return;
    };

    let normalized: Vec<Value> = arr
        .iter()
        .map(|element| normalize_list_item(item_template, element, depth + 1))
        .collect();

    values.set(key, Value::Array(normalized));
}

/// Normalize a single list item against the item template.
fn normalize_list_item(template: &Parameter, element: &Value, depth: u8) -> Value {
    if depth >= MAX_NORMALIZE_DEPTH {
        return element.clone();
    }

    // Wrap the element in a temporary ParameterValues keyed by the template id,
    // normalize, then extract the result.
    let mut temp = ParameterValues::new();
    temp.set(&template.id, element.clone());
    normalize_one(template, &mut temp, depth);
    temp.get(&template.id)
        .cloned()
        .unwrap_or_else(|| element.clone())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a JSON object map into a [`ParameterValues`].
fn object_to_parameter_values(obj: &serde_json::Map<String, Value>) -> ParameterValues {
    obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// Convert a [`ParameterValues`] back into a JSON object.
fn parameter_values_to_object(values: &ParameterValues) -> Value {
    let map: serde_json::Map<String, Value> = values
        .as_map()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // ── Helpers ─────────────────────────────────────────────────────────────

    fn make_values(pairs: &[(&str, Value)]) -> ParameterValues {
        let mut v = ParameterValues::new();
        for (k, val) in pairs {
            v.set(*k, val.clone());
        }
        v
    }

    // ── Basic default backfill ──────────────────────────────────────────────

    #[test]
    fn backfills_missing_string_default() {
        let params = vec![Parameter::string("name").default(json!("anon"))];
        let result = normalize_parameters(&params, &ParameterValues::new());
        assert_eq!(result.get("name"), Some(&json!("anon")));
    }

    #[test]
    fn preserves_existing_value() {
        let params = vec![Parameter::string("name").default(json!("anon"))];
        let values = make_values(&[("name", json!("Alice"))]);
        let result = normalize_parameters(&params, &values);
        assert_eq!(result.get("name"), Some(&json!("Alice")));
    }

    #[test]
    fn preserves_extra_keys() {
        let params = vec![Parameter::string("name").default(json!("anon"))];
        let values = make_values(&[("extra", json!(42))]);
        let result = normalize_parameters(&params, &values);
        assert_eq!(result.get("extra"), Some(&json!(42)));
        assert_eq!(result.get("name"), Some(&json!("anon")));
    }

    #[test]
    fn no_default_no_value_leaves_absent() {
        let params = vec![Parameter::string("name")];
        let result = normalize_parameters(&params, &ParameterValues::new());
        assert!(!result.contains("name"));
    }

    // ── Skip types ──────────────────────────────────────────────────────────

    #[test]
    fn skips_computed() {
        let params = vec![Parameter::computed("full_name").default(json!("should not appear"))];
        let result = normalize_parameters(&params, &ParameterValues::new());
        assert!(!result.contains("full_name"));
    }

    #[test]
    fn skips_notice() {
        let params = vec![Parameter::notice("info_banner").default(json!("should not appear"))];
        let result = normalize_parameters(&params, &ParameterValues::new());
        assert!(!result.contains("info_banner"));
    }

    #[test]
    fn skips_hidden_backfill() {
        let params = vec![Parameter::hidden("secret_id").default(json!("s3cr3t"))];
        let result = normalize_parameters(&params, &ParameterValues::new());
        assert!(!result.contains("secret_id"));
    }

    // ── Mode normalization ──────────────────────────────────────────────────

    #[test]
    fn mode_backfills_default_variant_when_absent() {
        let params = vec![
            Parameter::mode("auth")
                .variant(Parameter::string("bearer"))
                .variant(Parameter::string("basic"))
                .default_variant("bearer"),
        ];
        let result = normalize_parameters(&params, &ParameterValues::new());
        assert_eq!(result.get("auth"), Some(&json!({"mode": "bearer"})));
    }

    #[test]
    fn mode_preserves_existing_mode_value() {
        let params = vec![
            Parameter::mode("auth")
                .variant(Parameter::string("bearer").default(json!("tok")))
                .variant(Parameter::string("basic"))
                .default_variant("bearer"),
        ];
        let values = make_values(&[("auth", json!({"mode": "basic", "value": "creds"}))]);
        let result = normalize_parameters(&params, &values);
        assert_eq!(
            result.get("auth"),
            Some(&json!({"mode": "basic", "value": "creds"}))
        );
    }

    #[test]
    fn mode_backfills_variant_scalar_default() {
        let params = vec![
            Parameter::mode("auth")
                .variant(Parameter::string("bearer").default(json!("default_token")))
                .default_variant("bearer"),
        ];
        // Value has mode selected but no "value" key.
        let values = make_values(&[("auth", json!({"mode": "bearer"}))]);
        let result = normalize_parameters(&params, &values);
        assert_eq!(
            result.get("auth"),
            Some(&json!({"mode": "bearer", "value": "default_token"}))
        );
    }

    #[test]
    fn mode_backfills_mode_key_from_default_variant() {
        let params = vec![
            Parameter::mode("auth")
                .variant(Parameter::string("bearer"))
                .default_variant("bearer"),
        ];
        // Object without "mode" key.
        let values = make_values(&[("auth", json!({}))]);
        let result = normalize_parameters(&params, &values);
        let obj = result.get("auth").unwrap().as_object().unwrap();
        assert_eq!(obj.get("mode"), Some(&json!("bearer")));
    }

    // ── Object normalization ────────────────────────────────────────────────

    #[test]
    fn object_inline_backfills_nested_defaults() {
        let params = vec![
            Parameter::object("config")
                .add(Parameter::string("host").default(json!("localhost")))
                .add(Parameter::integer("port").default(json!(8080))),
        ];
        let values = make_values(&[("config", json!({}))]);
        let result = normalize_parameters(&params, &values);
        assert_eq!(
            result.get("config"),
            Some(&json!({"host": "localhost", "port": 8080}))
        );
    }

    #[test]
    fn object_inline_preserves_existing_nested_values() {
        let params = vec![
            Parameter::object("config")
                .add(Parameter::string("host").default(json!("localhost")))
                .add(Parameter::integer("port").default(json!(8080))),
        ];
        let values = make_values(&[("config", json!({"host": "example.com"}))]);
        let result = normalize_parameters(&params, &values);
        let obj = result.get("config").unwrap().as_object().unwrap();
        assert_eq!(obj.get("host"), Some(&json!("example.com")));
        assert_eq!(obj.get("port"), Some(&json!(8080)));
    }

    #[test]
    fn object_pick_mode_does_not_backfill_absent_fields() {
        let params = vec![
            Parameter::object("config")
                .pick_fields()
                .add(Parameter::string("host").default(json!("localhost")))
                .add(Parameter::integer("port").default(json!(8080))),
        ];
        let values = make_values(&[("config", json!({"host": "example.com"}))]);
        let result = normalize_parameters(&params, &values);
        let obj = result.get("config").unwrap().as_object().unwrap();
        assert_eq!(obj.get("host"), Some(&json!("example.com")));
        // Port was not "picked" — should NOT be backfilled.
        assert!(!obj.contains_key("port"));
    }

    #[test]
    fn object_sections_behaves_like_pick_mode() {
        let params = vec![
            Parameter::object("config")
                .sections()
                .add(Parameter::string("host").default(json!("localhost")))
                .add(Parameter::integer("port").default(json!(8080))),
        ];
        let values = make_values(&[("config", json!({}))]);
        let result = normalize_parameters(&params, &values);
        let obj = result.get("config").unwrap().as_object().unwrap();
        assert!(!obj.contains_key("host"));
        assert!(!obj.contains_key("port"));
    }

    // ── List normalization ──────────────────────────────────────────────────

    #[test]
    fn list_normalizes_each_item() {
        let params = vec![Parameter::list(
            "items",
            Parameter::object("item")
                .add(Parameter::string("name").default(json!("unnamed")))
                .add(Parameter::integer("qty").default(json!(1))),
        )];
        let values = make_values(&[(
            "items",
            json!([
                {"name": "Widget"},
                {},
            ]),
        )]);
        let result = normalize_parameters(&params, &values);
        let arr = result.get("items").unwrap().as_array().unwrap();
        assert_eq!(arr[0]["name"], json!("Widget"));
        assert_eq!(arr[0]["qty"], json!(1));
        assert_eq!(arr[1]["name"], json!("unnamed"));
        assert_eq!(arr[1]["qty"], json!(1));
    }

    #[test]
    fn list_preserves_non_array_value() {
        let params = vec![Parameter::list("items", Parameter::string("item"))];
        // Value is not an array — left unchanged.
        let values = make_values(&[("items", json!("not an array"))]);
        let result = normalize_parameters(&params, &values);
        assert_eq!(result.get("items"), Some(&json!("not an array")));
    }

    // ── Depth limit ─────────────────────────────────────────────────────────

    #[test]
    fn respects_depth_limit() {
        // Build a schema that's deeper than MAX_NORMALIZE_DEPTH.
        fn deep_object(depth: u8) -> Parameter {
            if depth == 0 {
                return Parameter::string("leaf").default(json!("deep"));
            }
            Parameter::object(&format!("level_{depth}")).add(deep_object(depth - 1))
        }

        let params = vec![deep_object(20)];
        let values = make_values(&[("level_20", json!({}))]);
        // Should not stack overflow — just stops backfilling at depth limit.
        let _result = normalize_parameters(&params, &values);
    }
}
