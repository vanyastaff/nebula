//! Object manipulation functions

use serde_json::Value;

use super::{check_arg_count, check_min_arg_count, get_array_arg, get_object_arg};
use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::{ExpressionErrorExt, ExpressionResult},
    eval::BuiltinView,
};

/// Get all keys of an object
pub fn keys(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("keys", args, 1)?;
    let obj = get_object_arg("keys", args, 0, "object")?;

    // Pre-allocate with known size to avoid reallocations
    let keys: Vec<_> = obj.keys().map(|k| Value::String(k.clone())).collect();

    Ok(Value::Array(keys))
}

/// Get all values of an object
pub fn values(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("values", args, 1)?;
    let obj = get_object_arg("values", args, 0, "object")?;

    // Collect into Vec directly - single allocation
    let values: Vec<_> = obj.values().cloned().collect();

    Ok(Value::Array(values))
}

/// Check if an object has a specific key
pub fn has(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("has", args, 2)?;
    let obj = get_object_arg("has", args, 0, "object")?;
    let key = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;

    Ok(Value::Bool(obj.contains_key(key)))
}

/// Shallow merge of multiple objects (right wins on key conflicts)
///
/// Example: `merge({a:1}, {b:2}, {a:3})` returns `{a:3, b:2}`
pub fn merge(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("merge", args, 1)?;

    let mut result = serde_json::Map::new();
    for (i, _) in args.iter().enumerate() {
        let obj = get_object_arg("merge", args, i, "object")?;
        for (k, v) in obj {
            result.insert(k.clone(), v.clone());
        }
    }

    Ok(Value::Object(result))
}

/// Return an object with only the specified keys
///
/// Example: `pick({a:1, b:2, c:3}, "a", "c")` returns `{a:1, c:3}`
pub fn pick(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("pick", args, 1)?;
    let obj = get_object_arg("pick", args, 0, "object")?;

    let mut keys_to_pick = Vec::with_capacity(args.len().saturating_sub(1));
    for arg in &args[1..] {
        match arg.as_str() {
            Some(s) => keys_to_pick.push(s),
            None => {
                return Err(ExpressionError::expression_type_error(
                    "string",
                    crate::value_utils::value_type_name(arg),
                ));
            },
        }
    }

    let result: serde_json::Map<String, Value> = obj
        .iter()
        .filter(|(k, _)| keys_to_pick.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    Ok(Value::Object(result))
}

/// Return an object without the specified keys
///
/// Example: `omit({a:1, b:2, c:3}, "b")` returns `{a:1, c:3}`
pub fn omit(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("omit", args, 1)?;
    let obj = get_object_arg("omit", args, 0, "object")?;

    let mut keys_to_omit = Vec::with_capacity(args.len().saturating_sub(1));
    for arg in &args[1..] {
        match arg.as_str() {
            Some(s) => keys_to_omit.push(s),
            None => {
                return Err(ExpressionError::expression_type_error(
                    "string",
                    crate::value_utils::value_type_name(arg),
                ));
            },
        }
    }

    let result: serde_json::Map<String, Value> = obj
        .iter()
        .filter(|(k, _)| !keys_to_omit.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    Ok(Value::Object(result))
}

/// Convert an object to an array of `{key, value}` pairs
///
/// Example: `entries({a:1, b:2})` returns `[{key:"a", value:1}, {key:"b", value:2}]`
pub fn entries(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("entries", args, 1)?;
    let obj = get_object_arg("entries", args, 0, "object")?;

    let result: Vec<Value> = obj
        .iter()
        .map(|(k, v)| {
            let mut pair = serde_json::Map::new();
            pair.insert("key".to_string(), Value::String(k.clone()));
            pair.insert("value".to_string(), v.clone());
            Value::Object(pair)
        })
        .collect();

    Ok(Value::Array(result))
}

/// Convert an array of `{key, value}` pairs back to an object
///
/// Example: `from_entries([{key:"a", value:1}])` returns `{a:1}`
pub fn from_entries(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("from_entries", args, 1)?;
    let arr = get_array_arg("from_entries", args, 0, "array")?;

    let mut result = serde_json::Map::new();
    for item in arr {
        let pair = item.as_object().ok_or_else(|| {
            ExpressionError::expression_invalid_argument(
                "from_entries",
                "Each element must be an object with 'key' and 'value' fields",
            )
        })?;

        let key = pair.get("key").and_then(|v| v.as_str()).ok_or_else(|| {
            ExpressionError::expression_invalid_argument(
                "from_entries",
                "Each element must have a string 'key' field",
            )
        })?;

        let value = pair.get("value").cloned().unwrap_or(Value::Null);
        result.insert(key.to_string(), value);
    }

    Ok(Value::Object(result))
}
