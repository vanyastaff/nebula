//! Type conversion functions

use super::check_arg_count;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;

/// Convert value to string
pub fn to_string(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_string", args, 1)?;
    Ok(Value::text(args[0].to_string()))
}

/// Convert value to number
pub fn to_number(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_number", args, 1)?;

    // Try integer first, then float
    if let Ok(int_val) = args[0].to_integer() {
        Ok(Value::integer(int_val))
    } else if let Ok(float_val) = args[0].to_float() {
        Ok(Value::float(float_val))
    } else {
        Err(NebulaError::expression_type_error(
            "convertible to number",
            args[0].kind().name(),
        ))
    }
}

/// Convert value to boolean
pub fn to_boolean(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_boolean", args, 1)?;
    Ok(Value::boolean(args[0].to_boolean()))
}

/// Convert value to JSON string
pub fn to_json(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_json", args, 1)?;

    use nebula_value::ValueRefExt;
    let json_string = serde_json::to_string(&args[0].to_json()).map_err(|e| {
        NebulaError::expression_eval_error(format!("Failed to serialize to JSON: {}", e))
    })?;

    Ok(Value::text(json_string))
}

/// Parse JSON string to value
pub fn parse_json(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("parse_json", args, 1)?;

    let json_str = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;

    use nebula_value::JsonValueExt;
    let json: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| NebulaError::expression_eval_error(format!("Failed to parse JSON: {}", e)))?;

    Ok(json.to_nebula_value_or_null())
}
