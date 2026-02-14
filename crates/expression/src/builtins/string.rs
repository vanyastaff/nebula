//! String manipulation functions

use super::{check_arg_count, check_min_arg_count, get_int_arg, get_string_arg};
use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use serde_json::Value;

/// Get the length of a string
pub fn length(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("length", args, 1)?;
    let s = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[0]),
        )
    })?;
    Ok(Value::Number((s.len() as i64).into()))
}

/// Convert string to uppercase
pub fn uppercase(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("uppercase", args, 1)?;
    let s = get_string_arg("uppercase", args, 0, "text")?;
    Ok(Value::String(s.to_uppercase()))
}

/// Convert string to lowercase
pub fn lowercase(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("lowercase", args, 1)?;
    let s = get_string_arg("lowercase", args, 0, "text")?;
    Ok(Value::String(s.to_lowercase()))
}

/// Trim whitespace from both ends of a string
pub fn trim(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("trim", args, 1)?;
    let s = get_string_arg("trim", args, 0, "text")?;
    Ok(Value::String(s.trim().to_string()))
}

/// Split a string by a delimiter
pub fn split(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("split", args, 2)?;
    let s = get_string_arg("split", args, 0, "text")?;
    let delimiter = get_string_arg("split", args, 1, "delimiter")?;

    let parts: Vec<_> = s
        .split(delimiter)
        .map(|s| Value::String(s.to_string()))
        .collect();
    Ok(Value::Array(parts))
}

/// Replace occurrences of a substring
pub fn replace(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("replace", args, 3)?;
    let s = get_string_arg("replace", args, 0, "text")?;
    let from = get_string_arg("replace", args, 1, "from")?;
    let to = get_string_arg("replace", args, 2, "to")?;

    Ok(Value::String(s.replace(from, to)))
}

/// Get a substring
pub fn substring(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("substring", args, 2)?;
    let s = get_string_arg("substring", args, 0, "text")?;
    let start = get_int_arg("substring", args, 1, "start")? as usize;
    let end = if args.len() > 2 {
        get_int_arg("substring", args, 2, "end")? as usize
    } else {
        s.len()
    };

    let chars: Vec<char> = s.chars().collect();
    let result: String = chars
        .get(start..end.min(chars.len()))
        .unwrap_or(&[])
        .iter()
        .collect();
    Ok(Value::String(result))
}

/// Check if string contains a substring
pub fn contains(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("contains", args, 2)?;
    let s = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[0]),
        )
    })?;
    let needle = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;

    Ok(Value::Bool(s.contains(needle)))
}

/// Check if string starts with a prefix
pub fn starts_with(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("starts_with", args, 2)?;
    let s = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[0]),
        )
    })?;
    let prefix = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;

    Ok(Value::Bool(s.starts_with(prefix)))
}

/// Check if string ends with a suffix
pub fn ends_with(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("ends_with", args, 2)?;
    let s = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[0]),
        )
    })?;
    let suffix = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(&args[1]),
        )
    })?;

    Ok(Value::Bool(s.ends_with(suffix)))
}
