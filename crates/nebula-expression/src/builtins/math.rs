//! Math functions

use super::{check_arg_count, check_min_arg_count, get_number_arg};
use crate::context::EvaluationContext;
use crate::core::error::{ExpressionErrorExt, ExpressionResult};
use crate::eval::Evaluator;
use nebula_error::NebulaError;
use nebula_value::Value;

/// Absolute value
pub fn abs(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_arg_count("abs", args, 1)?;
    let num = get_number_arg("abs", args, 0, "value")?;
    Ok(Value::float(num.abs()))
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
        Ok(Value::float(rounded))
    } else {
        // Round to nearest integer
        Ok(Value::float(num.round()))
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
    Ok(Value::float(num.floor()))
}

/// Ceiling function
pub fn ceil(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("ceil", args, 1)?;
    let num = args[0].to_float()?;
    Ok(Value::float(num.ceil()))
}

/// Minimum of two or more numbers
pub fn min(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_min_arg_count("min", args, 1)?;

    let mut min_val = args[0].to_float()?;
    for arg in &args[1..] {
        let val = arg.to_float()?;
        if val < min_val {
            min_val = val;
        }
    }

    Ok(Value::float(min_val))
}

/// Maximum of two or more numbers
pub fn max(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_min_arg_count("max", args, 1)?;

    let mut max_val = args[0].to_float()?;
    for arg in &args[1..] {
        let val = arg.to_float()?;
        if val > max_val {
            max_val = val;
        }
    }

    Ok(Value::float(max_val))
}

/// Square root
pub fn sqrt(
    args: &[Value],
    _eval: &Evaluator,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("sqrt", args, 1)?;
    let num = args[0].to_float()?;
    if num < 0.0 {
        return Err(NebulaError::expression_invalid_argument(
            "sqrt",
            "Cannot take square root of negative number",
        ));
    }
    Ok(Value::float(num.sqrt()))
}

/// Power function
pub fn pow(args: &[Value], _eval: &Evaluator, _ctx: &EvaluationContext) -> ExpressionResult<Value> {
    check_arg_count("pow", args, 2)?;
    let base = args[0].to_float()?;
    let exp = args[1].to_float()?;
    Ok(Value::float(base.powf(exp)))
}
