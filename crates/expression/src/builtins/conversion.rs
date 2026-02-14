//! Type conversion functions

use super::check_arg_count;
use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use serde_json::Value;

/// Maximum JSON string length to parse (1MB) - DoS protection
const MAX_JSON_PARSE_LENGTH: usize = 1024 * 1024;

/// Convert value to string
pub fn to_string(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_string", args, 1)?;
    let string_val = match &args[0] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(&args[0]).map_err(|e| {
            ExpressionError::expression_eval_error(format!("Failed to convert to string: {}", e))
        })?,
    };
    Ok(Value::String(string_val))
}

/// Convert value to number
pub fn to_number(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_number", args, 1)?;

    // Use the helper function from value_utils
    crate::value_utils::to_float(&args[0])
        .map(|f| serde_json::json!(f))
        .map_err(|_| {
            ExpressionError::expression_type_error(
                "convertible to number",
                crate::value_utils::value_type_name(&args[0]),
            )
        })
}

/// Convert value to boolean
pub fn to_boolean(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_boolean", args, 1)?;
    Ok(Value::Bool(crate::value_utils::to_boolean(&args[0])))
}

/// Convert value to JSON string
pub fn to_json(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_json", args, 1)?;

    let json_string = serde_json::to_string(&args[0]).map_err(|e| {
        ExpressionError::expression_eval_error(format!("Failed to serialize to JSON: {}", e))
    })?;

    Ok(Value::String(json_string))
}

/// Parse JSON string to value
pub fn parse_json(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("parse_json", args, 1)?;

    let json_str = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[0]),
        )
    })?;

    // DoS protection: limit JSON string size
    if json_str.len() > MAX_JSON_PARSE_LENGTH {
        return Err(ExpressionError::expression_eval_error(format!(
            "JSON string too large: {} bytes (max {} bytes)",
            json_str.len(),
            MAX_JSON_PARSE_LENGTH
        )));
    }

    let json: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
        ExpressionError::expression_eval_error(format!("Failed to parse JSON: {}", e))
    })?;

    Ok(json)
}
