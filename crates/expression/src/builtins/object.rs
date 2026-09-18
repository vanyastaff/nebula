//! Object manipulation functions

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

use super::{check_arg_count, check_min_arg_count, get_array_arg, get_object_arg, get_value_arg};

/// Get all keys of an object
pub(crate) fn keys(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("keys", args, 1)?;
    let object = get_object_arg("keys", args, 0, "object")?;

    let keys: Vec<RuntimeValue> = object
        .keys()
        .map(|key| RuntimeValue::String(Arc::clone(key)))
        .collect();

    Ok(RuntimeValue::Array(keys.into()))
}

/// Get all values of an object
pub(crate) fn values(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("values", args, 1)?;
    let object = get_object_arg("values", args, 0, "object")?;

    let values: Vec<RuntimeValue> = object.values().cloned().collect();

    Ok(RuntimeValue::Array(values.into()))
}

/// Check if an object has a specific key
pub(crate) fn has(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("has", args, 2)?;
    let object = get_object_arg("has", args, 0, "object")?;
    let key = args[1]
        .as_value()
        .and_then(RuntimeValue::as_str)
        .ok_or_else(|| {
            ExpressionError::type_error(
                "string",
                args[1]
                    .as_value()
                    .map_or("lambda", crate::value_utils::value_type_name),
            )
        })?;

    Ok(RuntimeValue::Bool(object.contains_key(key)))
}

/// Shallow merge of multiple objects (right wins on key conflicts)
///
/// Example: `merge({a:1}, {b:2}, {a:3})` returns `{a:3, b:2}`
pub(crate) fn merge(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("merge", args, 1)?;

    let mut merged: BTreeMap<&str, &RuntimeValue> = BTreeMap::new();
    for (index, _) in args.iter().enumerate() {
        let object = get_object_arg("merge", args, index, "object")?;
        merged.extend(object.iter().map(|(key, value)| (key.as_ref(), value)));
    }
    view.output_builder(context).preflight_object(merged)?;

    let mut result = BTreeMap::new();
    for (index, _) in args.iter().enumerate() {
        let object = get_object_arg("merge", args, index, "object")?;
        for (key, value) in object {
            result.insert(Arc::clone(key), value.clone());
        }
    }

    Ok(RuntimeValue::Object(Arc::new(result)))
}

/// Return an object with only the specified keys
///
/// Example: `pick({a:1, b:2, c:3}, "a", "c")` returns `{a:1, c:3}`
pub(crate) fn pick(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("pick", args, 1)?;
    let object = get_object_arg("pick", args, 0, "object")?;

    view.charge_work(object.len().saturating_add(args.len().saturating_sub(1)))?;
    let mut keys_to_pick = HashSet::with_capacity(args.len().saturating_sub(1));
    for index in 1..args.len() {
        let value = get_value_arg("pick", args, index, "key")?;
        match value.as_str() {
            Some(key) => {
                keys_to_pick.insert(key);
            },
            None => {
                return Err(ExpressionError::type_error(
                    "string",
                    crate::value_utils::value_type_name(value),
                ));
            },
        }
    }

    let selected = object
        .iter()
        .filter(|(key, _)| keys_to_pick.contains(key.as_ref()))
        .map(|(key, value)| (key.as_ref(), value));
    view.output_builder(context).preflight_object(selected)?;
    let result: BTreeMap<Arc<str>, RuntimeValue> = object
        .iter()
        .filter(|(key, _)| keys_to_pick.contains(key.as_ref()))
        .map(|(key, value)| (Arc::clone(key), value.clone()))
        .collect();

    Ok(RuntimeValue::Object(Arc::new(result)))
}

/// Return an object without the specified keys
///
/// Example: `omit({a:1, b:2, c:3}, "b")` returns `{a:1, c:3}`
pub(crate) fn omit(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("omit", args, 1)?;
    let object = get_object_arg("omit", args, 0, "object")?;

    view.charge_work(object.len().saturating_add(args.len().saturating_sub(1)))?;
    let mut keys_to_omit = HashSet::with_capacity(args.len().saturating_sub(1));
    for index in 1..args.len() {
        let value = get_value_arg("omit", args, index, "key")?;
        match value.as_str() {
            Some(key) => {
                keys_to_omit.insert(key);
            },
            None => {
                return Err(ExpressionError::type_error(
                    "string",
                    crate::value_utils::value_type_name(value),
                ));
            },
        }
    }

    let selected = object
        .iter()
        .filter(|(key, _)| !keys_to_omit.contains(key.as_ref()))
        .map(|(key, value)| (key.as_ref(), value));
    view.output_builder(context).preflight_object(selected)?;
    let result: BTreeMap<Arc<str>, RuntimeValue> = object
        .iter()
        .filter(|(key, _)| !keys_to_omit.contains(key.as_ref()))
        .map(|(key, value)| (Arc::clone(key), value.clone()))
        .collect();

    Ok(RuntimeValue::Object(Arc::new(result)))
}

/// Convert an object to an array of `{key, value}` pairs
///
/// Example: `entries({a:1, b:2})` returns `[{key:"a", value:1}, {key:"b", value:2}]`
pub(crate) fn entries(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("entries", args, 1)?;
    let object = get_object_arg("entries", args, 0, "object")?;
    view.output_builder(context).preflight_entries(object)?;

    let result: Vec<RuntimeValue> = object
        .iter()
        .map(|(key, value)| {
            let mut pair = BTreeMap::new();
            pair.insert(Arc::from("key"), RuntimeValue::String(Arc::clone(key)));
            pair.insert(Arc::from("value"), value.clone());
            RuntimeValue::Object(Arc::new(pair))
        })
        .collect();

    Ok(RuntimeValue::Array(result.into()))
}

/// Convert an array of `{key, value}` pairs back to an object
///
/// Example: `from_entries([{key:"a", value:1}])` returns `{a:1}`
pub(crate) fn from_entries(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("from_entries", args, 1)?;
    let array = get_array_arg("from_entries", args, 0, "array")?;

    let mut result = BTreeMap::new();
    for item in array {
        let pair = item.as_object().ok_or_else(|| {
            ExpressionError::invalid_argument(
                "from_entries",
                "Each element must be an object with 'key' and 'value' fields",
            )
        })?;

        let key = pair
            .get("key")
            .and_then(RuntimeValue::as_str)
            .ok_or_else(|| {
                ExpressionError::invalid_argument(
                    "from_entries",
                    "Each element must have a string 'key' field",
                )
            })?;

        let value = pair.get("value").cloned().unwrap_or(RuntimeValue::Null);
        result.insert(Arc::from(key), value);
    }

    Ok(RuntimeValue::Object(Arc::new(result)))
}
