//! Utility functions

use serde_json::Value;

use super::{check_arg_count, check_min_arg_count};
use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::{ExpressionErrorExt, ExpressionResult},
    eval::BuiltinView,
};

/// Get the length of a string, array, or object — the single polymorphic
/// `length()` exposed to expressions.
///
/// - **String**: Unicode scalar values (`chars`), matching JS / n8n semantics — `length("🙂")` is
///   1, not 4.
/// - **Array**: number of elements.
/// - **Object**: number of top-level keys.
///
/// All other input types yield a typed error.
pub fn length(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("length", args, 1)?;
    match &args[0] {
        Value::String(t) => Ok(Value::Number(crate::value_utils::char_count(t).into())),
        Value::Array(arr) => Ok(Value::Number((arr.len() as i64).into())),
        Value::Object(obj) => Ok(Value::Number((obj.len() as i64).into())),
        _ => Err(ExpressionError::expression_type_error(
            "string, array, or object",
            crate::value_utils::value_type_name(&args[0]),
        )),
    }
}

/// Check if value is null
pub fn is_null(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_null", args, 1)?;
    Ok(Value::Bool(args[0].is_null()))
}

/// Check if value is an array
pub fn is_array(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_array", args, 1)?;
    Ok(Value::Bool(args[0].is_array()))
}

/// Check if value is an object
pub fn is_object(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_object", args, 1)?;
    Ok(Value::Bool(args[0].is_object()))
}

/// Check if value is a string
pub fn is_string(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_string", args, 1)?;
    Ok(Value::Bool(args[0].is_string()))
}

/// Check if value is a number
pub fn is_number(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("is_number", args, 1)?;
    Ok(Value::Bool(args[0].is_number()))
}

/// Generate a new UUID
#[cfg(feature = "uuid")]
pub fn uuid(
    _args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    let id = uuid::Uuid::new_v4();
    Ok(Value::String(id.to_string()))
}

/// Generate a new UUID (fallback when feature disabled)
#[cfg(not(feature = "uuid"))]
pub fn uuid(
    _args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(ExpressionError::expression_function_not_found(
        "uuid (feature 'uuid' not enabled)",
    ))
}

/// Return the first non-null value from the arguments
///
/// Example: `coalesce(null, null, 42, "hello")` returns `42`
pub fn coalesce(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("coalesce", args, 1)?;

    for arg in args {
        if !arg.is_null() {
            return Ok(arg.clone());
        }
    }

    Ok(Value::Null)
}

/// Return the type name of a value as a string
///
/// Example: `type_of(42)` returns `"number"`
pub fn type_of(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("type_of", args, 1)?;
    Ok(Value::String(
        crate::value_utils::value_type_name(&args[0]).to_string(),
    ))
}
