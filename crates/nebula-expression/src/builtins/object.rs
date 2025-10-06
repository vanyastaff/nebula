//! Object manipulation functions

use super::check_arg_count;
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
    let obj = args[0]
        .as_object()
        .ok_or_else(|| NebulaError::expression_type_error("object", args[0].kind().name()))?;

    use nebula_value::ValueRefExt;
    let mut result = nebula_value::Array::new();
    for key in obj.keys() {
        result = result.push(Value::text(key).to_json());
    }
    Ok(Value::Array(result))
}

/// Get all values of an object
pub fn values(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("values", args, 1)?;
    let obj = args[0]
        .as_object()
        .ok_or_else(|| NebulaError::expression_type_error("object", args[0].kind().name()))?;

    let mut result = nebula_value::Array::new();
    for value in obj.values() {
        result.push(value.clone());
    }
    Ok(Value::Array(result))
}

/// Check if an object has a specific key
pub fn has(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_arg_count("has", args, 2)?;
    let obj = args[0]
        .as_object()
        .ok_or_else(|| NebulaError::expression_type_error("object", args[0].kind().name()))?;
    let key = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    Ok(Value::boolean(obj.contains_key(key)))
}
