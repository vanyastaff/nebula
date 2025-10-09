//! Utility functions

use super::check_arg_count;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;

/// Get the length of a string or array
pub fn length(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("length", args, 1)?;
    match &args[0] {
        Value::Text(t) => Ok(Value::integer(t.len() as i64)),
        Value::Array(arr) => Ok(Value::integer(arr.len() as i64)),
        _ => Err(NebulaError::expression_type_error(
            "string or array",
            args[0].kind().name(),
        )),
    }
}

/// Check if value is null
pub fn is_null(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_null", args, 1)?;
    Ok(Value::boolean(args[0].is_null()))
}

/// Check if value is an array
pub fn is_array(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_array", args, 1)?;
    Ok(Value::boolean(args[0].is_array()))
}

/// Check if value is an object
pub fn is_object(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_object", args, 1)?;
    Ok(Value::boolean(args[0].is_object()))
}

/// Check if value is a string
pub fn is_string(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_string", args, 1)?;
    Ok(Value::boolean(args[0].is_text()))
}

/// Check if value is a number
pub fn is_number(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_number", args, 1)?;
    Ok(Value::boolean(args[0].is_numeric()))
}

/// Generate a new UUID
#[cfg(feature = "uuid")]
pub fn uuid(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    let id = uuid::Uuid::new_v4();
    Ok(Value::text(id.to_string()))
}

/// Generate a new UUID (fallback when feature disabled)
#[cfg(not(feature = "uuid"))]
pub fn uuid(
    _args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(nebula_error::NebulaError::expression_function_not_found(
        "uuid (feature 'uuid' not enabled)",
    ))
}
