//! Object manipulation functions

use super::{check_arg_count, get_object_arg};
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;

/// Get all keys of an object
pub fn keys(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("keys", args, 1)?;
    let obj = get_object_arg("keys", args, 0, "object")?;

    // Pre-allocate with known size to avoid reallocations
    let keys: Vec<_> = obj
        .keys()
        .map(|k| Value::text(k.to_string()))
        .collect();

    Ok(Value::Array(nebula_value::Array::from_vec(keys)))
}

/// Get all values of an object
pub fn values(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("values", args, 1)?;
    let obj = get_object_arg("values", args, 0, "object")?;

    // Collect into Vec directly - single allocation
    let values: Vec<_> = obj.values().cloned().collect();

    Ok(Value::Array(nebula_value::Array::from_vec(values)))
}

/// Check if an object has a specific key
pub fn has(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_arg_count("has", args, 2)?;
    let obj = get_object_arg("has", args, 0, "object")?;
    let key = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    Ok(Value::boolean(obj.contains_key(key)))
}
