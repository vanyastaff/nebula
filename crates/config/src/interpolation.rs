//! Environment variable interpolation for configuration values.
//!
//! Supports `${VAR}` (required) and `${VAR:-default}` (with fallback) syntax.
//! Walks JSON trees recursively and only interpolates `Value::String` leaves.
//! A literal `$$` is treated as an escaped `$` and is not interpolated.

use serde_json::Value;

use crate::core::error::ConfigError;
use crate::core::result::ConfigResult;

/// Interpolate environment variable references in a JSON value tree.
///
/// Walks the tree recursively. Only `Value::String` leaves are processed.
/// Supports:
/// - `${VAR}` — resolves from the environment; errors if unset
/// - `${VAR:-default}` — resolves from the environment; uses *default* if unset
/// - `$$` — escaped literal `$` (not interpolated)
///
/// # Errors
///
/// Returns [`ConfigError::InterpolationError`] when a required variable is
/// missing or the syntax is invalid (e.g. empty `${}`).
pub fn interpolate(value: &Value) -> ConfigResult<Value> {
    match value {
        Value::String(s) => interpolate_string(s),
        Value::Object(map) => {
            let mut result = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                result.insert(k.clone(), interpolate(v)?);
            }
            Ok(Value::Object(result))
        }
        Value::Array(arr) => {
            let result: ConfigResult<Vec<Value>> = arr.iter().map(interpolate).collect();
            Ok(Value::Array(result?))
        }
        other => Ok(other.clone()),
    }
}

/// Interpolate a single string, resolving all `${…}` references.
fn interpolate_string(input: &str) -> ConfigResult<Value> {
    // Fast path: no `$` at all → return unchanged.
    if !input.contains('$') {
        return Ok(Value::String(input.to_string()));
    }

    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut ref_count: u32 = 0;

    while let Some(ch) = chars.next() {
        if ch == '$' {
            match chars.peek() {
                Some('$') => {
                    // Escaped `$$` → literal `$`
                    chars.next();
                    result.push('$');
                }
                Some('{') => {
                    chars.next(); // consume '{'
                    let (resolved, key) = resolve_reference(&mut chars)?;
                    result.push_str(&resolved);
                    ref_count += 1;
                    nebula_log::trace!("resolved ${{{key}}} from environment");
                }
                _ => {
                    // Bare `$` — pass through literally
                    result.push('$');
                }
            }
        } else {
            result.push(ch);
        }
    }

    if ref_count > 0 {
        nebula_log::debug!(
            "interpolating config value: found {count} variable references",
            count = ref_count
        );
    }

    Ok(Value::String(result))
}

/// Parse a `${…}` reference body from a char iterator and resolve it.
///
/// Expects the iterator is positioned right after `${`.
/// Returns `(resolved_value, var_name)`.
fn resolve_reference(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> ConfigResult<(String, String)> {
    let mut body = String::new();
    let mut found_close = false;

    for ch in chars.by_ref() {
        if ch == '}' {
            found_close = true;
            break;
        }
        body.push(ch);
    }

    if !found_close {
        return Err(ConfigError::interpolation_error(
            "unclosed variable reference — missing '}'",
            None,
        ));
    }

    if body.is_empty() {
        return Err(ConfigError::interpolation_error(
            "empty variable reference ${}",
            None,
        ));
    }

    // Split on `:-` for default syntax: ${VAR:-default}
    let (var_name, default_value) = if let Some(pos) = body.find(":-") {
        let name = &body[..pos];
        let fallback = &body[pos + 2..];
        (name.to_string(), Some(fallback.to_string()))
    } else {
        (body, None)
    };

    match std::env::var(&var_name) {
        Ok(val) => Ok((val, var_name)),
        Err(_) => {
            if let Some(default) = default_value {
                nebula_log::warn!("unresolved variable ${{{var_name}}}, using default: {default}");
                Ok((default, var_name))
            } else {
                Err(ConfigError::interpolation_error(
                    format!("unresolved environment variable: {var_name}"),
                    Some(var_name),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_env_variable() {
        unsafe { std::env::set_var("NEBULA_TEST_INTERP_A", "hello") };
        let input = json!({"greeting": "${NEBULA_TEST_INTERP_A}"});
        let result = interpolate(&input).unwrap();
        assert_eq!(result["greeting"], "hello");
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_A") };
    }

    #[test]
    fn fallback_when_var_unset() {
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_UNSET") };
        let input = json!({"val": "${NEBULA_TEST_INTERP_UNSET:-fallback_value}"});
        let result = interpolate(&input).unwrap();
        assert_eq!(result["val"], "fallback_value");
    }

    #[test]
    fn fallback_ignored_when_var_set() {
        unsafe { std::env::set_var("NEBULA_TEST_INTERP_B", "real") };
        let input = json!({"val": "${NEBULA_TEST_INTERP_B:-ignored}"});
        let result = interpolate(&input).unwrap();
        assert_eq!(result["val"], "real");
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_B") };
    }

    #[test]
    fn multiple_vars_in_one_string() {
        unsafe {
            std::env::set_var("NEBULA_TEST_INTERP_X", "foo");
            std::env::set_var("NEBULA_TEST_INTERP_Y", "bar");
        }
        let input = json!({"val": "${NEBULA_TEST_INTERP_X}_${NEBULA_TEST_INTERP_Y}"});
        let result = interpolate(&input).unwrap();
        assert_eq!(result["val"], "foo_bar");
        unsafe {
            std::env::remove_var("NEBULA_TEST_INTERP_X");
            std::env::remove_var("NEBULA_TEST_INTERP_Y");
        }
    }

    #[test]
    fn non_string_values_pass_through() {
        let input = json!({"num": 42, "bool": true, "null": null});
        let result = interpolate(&input).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn deep_nested_interpolation() {
        unsafe { std::env::set_var("NEBULA_TEST_INTERP_DEEP", "found") };
        let input = json!({
            "level1": {
                "level2": {
                    "items": ["${NEBULA_TEST_INTERP_DEEP}", "static"]
                }
            }
        });
        let result = interpolate(&input).unwrap();
        assert_eq!(result["level1"]["level2"]["items"][0], "found");
        assert_eq!(result["level1"]["level2"]["items"][1], "static");
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_DEEP") };
    }

    #[test]
    fn missing_required_var_returns_error() {
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_MISSING") };
        let input = json!({"val": "${NEBULA_TEST_INTERP_MISSING}"});
        let err = interpolate(&input).unwrap_err();
        assert!(matches!(err, ConfigError::InterpolationError { .. }));
    }

    #[test]
    fn empty_reference_returns_error() {
        let input = json!({"val": "${}"});
        let err = interpolate(&input).unwrap_err();
        assert!(matches!(err, ConfigError::InterpolationError { .. }));
    }

    #[test]
    fn escaped_dollar_not_interpolated() {
        let input = json!({"val": "price is $$10"});
        let result = interpolate(&input).unwrap();
        assert_eq!(result["val"], "price is $10");
    }
}
