//! Math functions

use super::{check_arg_count, check_min_arg_count, get_number_arg};
use crate::ExpressionError;
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use serde_json::Value;

/// Absolute value
pub fn abs(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_arg_count("abs", args, 1)?;
    let num = get_number_arg("abs", args, 0, "value")?;
    Ok(serde_json::json!(num.abs()))
}

/// Round to specified decimal places (default: 0)
pub fn round(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("round", args, 1)?;
    let num = get_number_arg("round", args, 0, "value")?;

    if args.len() >= 2 {
        // Round to specific decimal places
        let decimals = super::get_int_arg("round", args, 1, "decimals")? as u32;
        let multiplier = 10_f64.powi(decimals as i32);
        let rounded = (num * multiplier).round() / multiplier;
        Ok(serde_json::json!(rounded))
    } else {
        // Round to nearest integer
        Ok(serde_json::json!(num.round()))
    }
}

/// Floor function
pub fn floor(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("floor", args, 1)?;
    let num = get_number_arg("floor", args, 0, "value")?;
    Ok(serde_json::json!(num.floor()))
}

/// Ceiling function
pub fn ceil(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("ceil", args, 1)?;
    let num = get_number_arg("ceil", args, 0, "value")?;
    Ok(serde_json::json!(num.ceil()))
}

/// Minimum of two or more numbers
pub fn min(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_min_arg_count("min", args, 1)?;

    let mut min_val = get_number_arg("min", args, 0, "value")?;
    for (i, arg) in args[1..].iter().enumerate() {
        let val = crate::value_utils::to_float(arg).map_err(|_| {
            ExpressionError::expression_invalid_argument(
                "min",
                format!("Argument at position {} must be a number", i + 1),
            )
        })?;
        if val < min_val {
            min_val = val;
        }
    }

    Ok(serde_json::json!(min_val))
}

/// Maximum of two or more numbers
pub fn max(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_min_arg_count("max", args, 1)?;

    let mut max_val = get_number_arg("max", args, 0, "value")?;
    for (i, arg) in args[1..].iter().enumerate() {
        let val = crate::value_utils::to_float(arg).map_err(|_| {
            ExpressionError::expression_invalid_argument(
                "max",
                format!("Argument at position {} must be a number", i + 1),
            )
        })?;
        if val > max_val {
            max_val = val;
        }
    }

    Ok(serde_json::json!(max_val))
}

/// Square root
pub fn sqrt(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("sqrt", args, 1)?;
    let num = get_number_arg("sqrt", args, 0, "value")?;
    if num < 0.0 {
        return Err(ExpressionError::expression_invalid_argument(
            "sqrt",
            "Cannot take square root of negative number",
        ));
    }
    Ok(serde_json::json!(num.sqrt()))
}

/// Power function
pub fn pow(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_arg_count("pow", args, 2)?;
    let base = get_number_arg("pow", args, 0, "base")?;
    let exp = get_number_arg("pow", args, 1, "exponent")?;
    Ok(serde_json::json!(base.powf(exp)))
}
