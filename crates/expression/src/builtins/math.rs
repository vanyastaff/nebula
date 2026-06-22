//! Math functions

use serde_json::Value;

use super::{
    check_arg_count, check_min_arg_count, get_int_arg_with_policy, get_number_arg_with_policy,
};
use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::{ExpressionErrorExt, ExpressionResult},
    eval::BuiltinView,
};

/// Wrap a computed `f64` result, rejecting a non-finite value with a typed error
/// instead of letting `serde_json` turn `inf`/`NaN` silently into `null`.
///
/// Mirrors the `**` operator's finiteness guard so the function and operator
/// forms agree.
fn finite_result(fn_name: &str, value: f64) -> ExpressionResult<Value> {
    if value.is_finite() {
        Ok(serde_json::json!(value))
    } else {
        Err(ExpressionError::expression_invalid_argument(
            fn_name,
            "result is not a finite number",
        ))
    }
}

/// Compare two JSON numbers by value, exactly for integers.
///
/// `f64` loses precision past 2^53, which would make `max`/`min` pick the wrong
/// 64-bit value; compare as `i64` (or `u64` when both exceed `i64::MAX`) and only
/// fall back to `f64` for genuine floats. Both inputs are validated as numbers by
/// the caller, so the `f64` fallback is total.
fn number_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if let (Some(lhs), Some(rhs)) = (a.as_i64(), b.as_i64()) {
        return lhs.cmp(&rhs);
    }
    if let (Some(lhs), Some(rhs)) = (a.as_u64(), b.as_u64()) {
        return lhs.cmp(&rhs);
    }
    let lhs = a.as_f64().unwrap_or(f64::NAN);
    let rhs = b.as_f64().unwrap_or(f64::NAN);
    lhs.partial_cmp(&rhs).unwrap_or(Ordering::Equal)
}

/// Absolute value
pub fn abs(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("abs", args, 1)?;
    let num = get_number_arg_with_policy("abs", args, 0, "value", view, ctx)?;
    Ok(serde_json::json!(num.abs()))
}

/// Round to specified decimal places (default: 0)
pub fn round(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("round", args, 1)?;
    let num = get_number_arg_with_policy("round", args, 0, "value", view, ctx)?;

    if args.len() >= 2 {
        // Round to specific decimal places
        let decimals = get_int_arg_with_policy("round", args, 1, "decimals", view, ctx)?;
        if decimals < 0 {
            return Err(ExpressionError::expression_invalid_argument(
                "round",
                "Argument 'decimals' must be non-negative",
            ));
        }
        let decimals = decimals as u32;
        let multiplier = 10_f64.powi(decimals as i32);
        let rounded = (num * multiplier).round() / multiplier;
        // A very large `decimals` makes `multiplier` overflow to `inf`, turning
        // `rounded` into `NaN`; reject that instead of returning silent `null`.
        finite_result("round", rounded)
    } else {
        // Round to nearest integer
        Ok(serde_json::json!(num.round()))
    }
}

/// Floor function
pub fn floor(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("floor", args, 1)?;
    let num = get_number_arg_with_policy("floor", args, 0, "value", view, ctx)?;
    Ok(serde_json::json!(num.floor()))
}

/// Ceiling function
pub fn ceil(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("ceil", args, 1)?;
    let num = get_number_arg_with_policy("ceil", args, 0, "value", view, ctx)?;
    Ok(serde_json::json!(num.ceil()))
}

/// Minimum of two or more numbers
pub fn min(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("min", args, 1)?;

    // Validate arg 0 is numeric, then select by EXACT comparison and return the
    // original value — preserving integer type and exact 64-bit magnitude (an
    // f64 running min would collapse large ids and emit a spurious `.0`).
    get_number_arg_with_policy("min", args, 0, "value", view, ctx)?;
    let mut best = &args[0];
    for (i, arg) in args[1..].iter().enumerate() {
        get_number_arg_with_policy("min", std::slice::from_ref(arg), 0, "value", view, ctx)
            .map_err(|_| {
                ExpressionError::expression_invalid_argument(
                    "min",
                    format!("Argument at position {} must be a number", i + 1),
                )
            })?;
        if number_cmp(arg, best) == std::cmp::Ordering::Less {
            best = arg;
        }
    }

    Ok(best.clone())
}

/// Maximum of two or more numbers
pub fn max(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("max", args, 1)?;

    // See `min`: select by exact comparison and return the original value.
    get_number_arg_with_policy("max", args, 0, "value", view, ctx)?;
    let mut best = &args[0];
    for (i, arg) in args[1..].iter().enumerate() {
        get_number_arg_with_policy("max", std::slice::from_ref(arg), 0, "value", view, ctx)
            .map_err(|_| {
                ExpressionError::expression_invalid_argument(
                    "max",
                    format!("Argument at position {} must be a number", i + 1),
                )
            })?;
        if number_cmp(arg, best) == std::cmp::Ordering::Greater {
            best = arg;
        }
    }

    Ok(best.clone())
}

/// Square root
pub fn sqrt(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("sqrt", args, 1)?;
    let num = get_number_arg_with_policy("sqrt", args, 0, "value", view, ctx)?;
    if num < 0.0 {
        return Err(ExpressionError::expression_invalid_argument(
            "sqrt",
            "Cannot take square root of negative number",
        ));
    }
    Ok(serde_json::json!(num.sqrt()))
}

/// Power function
pub fn pow(
    args: &[Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("pow", args, 2)?;
    let base = get_number_arg_with_policy("pow", args, 0, "base", view, ctx)?;
    let exp = get_number_arg_with_policy("pow", args, 1, "exponent", view, ctx)?;
    // `powf` can overflow to `inf` (e.g. `pow(2, 1024)`) or be `NaN` (e.g.
    // `pow(-1, 0.5)`); reject both like the `**` operator does, rather than
    // emitting silent `null`.
    finite_result("pow", base.powf(exp))
}
