//! String manipulation functions

use super::{check_arg_count, check_min_arg_count};
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;

/// Get the length of a string
pub fn length(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("length", args, 1)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    Ok(Value::integer(s.len() as i64))
}

/// Convert string to uppercase
pub fn uppercase(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("uppercase", args, 1)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    Ok(Value::text(s.to_uppercase()))
}

/// Convert string to lowercase
pub fn lowercase(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("lowercase", args, 1)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    Ok(Value::text(s.to_lowercase()))
}

/// Trim whitespace from both ends of a string
pub fn trim(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("trim", args, 1)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    Ok(Value::text(s.trim()))
}

/// Split a string by a delimiter
pub fn split(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("split", args, 2)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    let delimiter = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    use nebula_value::ValueRefExt;
    let mut arr = nebula_value::Array::new();
    for part in s.split(delimiter) {
        arr = arr.push(Value::text(part).to_json());
    }
    Ok(Value::Array(arr))
}

/// Replace occurrences of a substring
pub fn replace(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("replace", args, 3)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    let from = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;
    let to = args[2]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[2].kind().name()))?;

    Ok(Value::text(s.replace(from, to)))
}

/// Get a substring
pub fn substring(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("substring", args, 2)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    let start = args[1].to_integer()? as usize;
    let end = if args.len() > 2 {
        args[2].to_integer()? as usize
    } else {
        s.len()
    };

    let chars: Vec<char> = s.chars().collect();
    let result: String = chars
        .get(start..end.min(chars.len()))
        .unwrap_or(&[])
        .iter()
        .collect();
    Ok(Value::text(result))
}

/// Check if string contains a substring
pub fn contains(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("contains", args, 2)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    let needle = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    Ok(Value::boolean(s.contains(needle)))
}

/// Check if string starts with a prefix
pub fn starts_with(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("starts_with", args, 2)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    let prefix = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    Ok(Value::boolean(s.starts_with(prefix)))
}

/// Check if string ends with a suffix
pub fn ends_with(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("ends_with", args, 2)?;
    let s = args[0]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[0].kind().name()))?;
    let suffix = args[1]
        .as_str()
        .ok_or_else(|| NebulaError::expression_type_error("string", args[1].kind().name()))?;

    Ok(Value::boolean(s.ends_with(suffix)))
}
