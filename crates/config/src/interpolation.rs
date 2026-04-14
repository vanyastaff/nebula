//! Environment variable interpolation for configuration values.
//!
//! Supports `${VAR}` (required) and `${VAR:-default}` (with fallback) syntax.
//! Walks JSON trees recursively and only interpolates `Value::String` leaves.
//! A literal `$$` is treated as an escaped `$` and is not interpolated.

use serde_json::Value;

use crate::core::{error::ConfigError, result::ConfigResult};

/// Interpolate environment variable references in a JSON value tree.
///
/// Walks the tree recursively. Only `Value::String` leaves are processed.
/// Supports:
/// - `${VAR}` — resolves from the environment; errors if unset
/// - `${VAR:-default}` — resolves from the environment; uses *default* if unset
/// - `$$` — escaped literal `$` (not interpolated)
///
/// Takes ownership of the value and returns it with no new heap allocations
/// when no `$` references are found anywhere in the tree.
///
/// # Errors
///
/// Returns [`ConfigError::InterpolationError`] when a required variable is
/// missing or the syntax is invalid (e.g. empty `${}`).
pub fn interpolate(value: Value) -> ConfigResult<Value> {
    match value {
        Value::String(s) => interpolate_string(s),
        Value::Object(mut map) => {
            // On error, the partially-mutated map/arr is dropped along with the Err —
            // the Value::Null placeholder is never observable to callers.
            for v in map.values_mut() {
                let owned = std::mem::replace(v, Value::Null);
                *v = interpolate(owned)?;
            }
            Ok(Value::Object(map))
        },
        Value::Array(mut arr) => {
            // On error, the partially-mutated map/arr is dropped along with the Err —
            // the Value::Null placeholder is never observable to callers.
            for v in arr.iter_mut() {
                let owned = std::mem::replace(v, Value::Null);
                *v = interpolate(owned)?;
            }
            Ok(Value::Array(arr))
        },
        other => Ok(other),
    }
}

/// Interpolate a single string, resolving all `${…}` references.
///
/// Takes ownership of the `String`. Returns it unchanged (reusing its heap
/// allocation) when no `$` is present — zero extra allocations on the fast path.
fn interpolate_string(input: String) -> ConfigResult<Value> {
    // Fast path: no `$` at all → return the original String without any allocation.
    let bytes = input.as_bytes();
    let Some(first_dollar) = bytes.iter().position(|&b| b == b'$') else {
        return Ok(Value::String(input));
    };

    // Slow path: found at least one `$`. Build the result string.
    // Copy everything before the first `$` upfront to avoid repeated checks.
    let mut result = String::with_capacity(input.len());
    result.push_str(&input[..first_dollar]);

    let mut pos = first_dollar;
    let mut ref_count: u32 = 0;

    while pos < bytes.len() {
        match bytes[pos] {
            b'$' => match bytes.get(pos + 1) {
                Some(b'$') => {
                    // Escaped `$$` → literal `$`
                    result.push('$');
                    pos += 2;
                },
                Some(b'{') => {
                    pos += 2; // consume '${'
                    let (resolved, key, new_pos) = resolve_reference(&input, pos)?;
                    result.push_str(&resolved);
                    ref_count += 1;
                    nebula_log::trace!("resolved ${{{key}}} from environment");
                    pos = new_pos;
                },
                _ => {
                    // Bare `$` — pass through literally
                    result.push('$');
                    pos += 1;
                },
            },
            _ => {
                // Bulk-copy everything up to the next `$` in one slice operation,
                // avoiding per-character processing for literal text segments.
                let next_dollar = bytes[pos..]
                    .iter()
                    .position(|&b| b == b'$')
                    .map(|p| pos + p)
                    .unwrap_or(bytes.len());
                result.push_str(&input[pos..next_dollar]);
                pos = next_dollar;
            },
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

/// Parse a `${…}` reference body from the input string and resolve it.
///
/// `start` is the byte offset just after `${`.
///
/// Returns `(resolved_value, var_name, new_pos)` where `new_pos` is the byte
/// offset immediately after the closing `}`.
fn resolve_reference(input: &str, start: usize) -> ConfigResult<(String, String, usize)> {
    let bytes = input.as_bytes();

    // Byte scan for `}` — all relevant characters are ASCII so this is safe
    // on valid UTF-8 (multi-byte sequences always have the high bit set).
    let close_offset = bytes[start..]
        .iter()
        .position(|&b| b == b'}')
        .ok_or_else(|| {
            ConfigError::interpolation_error("unclosed variable reference — missing '}'", None)
        })?;

    let body = &input[start..start + close_offset];
    let new_pos = start + close_offset + 1; // step past '}'

    if body.is_empty() {
        return Err(ConfigError::interpolation_error(
            "empty variable reference ${}",
            None,
        ));
    }

    // Split on `:-` for default syntax: ${VAR:-default}
    let (var_name, default_value) = if let Some(pos) = body.find(":-") {
        (&body[..pos], Some(&body[pos + 2..]))
    } else {
        (body, None)
    };

    match std::env::var(var_name) {
        Ok(val) => Ok((val, var_name.to_string(), new_pos)),
        Err(_) => {
            if let Some(default) = default_value {
                // Deliberately do NOT include the default value in the log:
                // defaults are the very place operators put fallback secrets
                // (e.g. `${DB_PASSWORD:-dev-password}`), and this warning goes
                // to tracing.
                nebula_log::warn!("unresolved variable ${{{var_name}}}, using configured default");
                Ok((default.to_owned(), var_name.to_string(), new_pos))
            } else {
                Err(ConfigError::interpolation_error(
                    format!("unresolved environment variable: {var_name}"),
                    Some(var_name.to_string()),
                ))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn resolves_env_variable() {
        unsafe { std::env::set_var("NEBULA_TEST_INTERP_A", "hello") };
        let input = json!({"greeting": "${NEBULA_TEST_INTERP_A}"});
        let result = interpolate(input).unwrap();
        assert_eq!(result["greeting"], "hello");
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_A") };
    }

    #[test]
    fn fallback_when_var_unset() {
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_UNSET") };
        let input = json!({"val": "${NEBULA_TEST_INTERP_UNSET:-fallback_value}"});
        let result = interpolate(input).unwrap();
        assert_eq!(result["val"], "fallback_value");
    }

    #[test]
    fn fallback_ignored_when_var_set() {
        unsafe { std::env::set_var("NEBULA_TEST_INTERP_B", "real") };
        let input = json!({"val": "${NEBULA_TEST_INTERP_B:-ignored}"});
        let result = interpolate(input).unwrap();
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
        let result = interpolate(input).unwrap();
        assert_eq!(result["val"], "foo_bar");
        unsafe {
            std::env::remove_var("NEBULA_TEST_INTERP_X");
            std::env::remove_var("NEBULA_TEST_INTERP_Y");
        }
    }

    #[test]
    fn non_string_values_pass_through() {
        let input = json!({"num": 42, "bool": true, "null": null});
        let result = interpolate(input.clone()).unwrap();
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
        let result = interpolate(input).unwrap();
        assert_eq!(result["level1"]["level2"]["items"][0], "found");
        assert_eq!(result["level1"]["level2"]["items"][1], "static");
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_DEEP") };
    }

    #[test]
    fn missing_required_var_returns_error() {
        unsafe { std::env::remove_var("NEBULA_TEST_INTERP_MISSING") };
        let input = json!({"val": "${NEBULA_TEST_INTERP_MISSING}"});
        let err = interpolate(input).unwrap_err();
        assert!(matches!(err, ConfigError::InterpolationError { .. }));
    }

    #[test]
    fn empty_reference_returns_error() {
        let input = json!({"val": "${}"});
        let err = interpolate(input).unwrap_err();
        assert!(matches!(err, ConfigError::InterpolationError { .. }));
    }

    #[test]
    fn escaped_dollar_not_interpolated() {
        let input = json!({"val": "price is $$10"});
        let result = interpolate(input).unwrap();
        assert_eq!(result["val"], "price is $10");
    }
}
