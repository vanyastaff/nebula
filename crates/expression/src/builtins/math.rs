//! Math functions

use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

use super::{
    check_arg_count, check_min_arg_count, get_int_arg_with_policy, get_number_arg_with_policy,
    get_value_arg,
};

/// Wrap a computed `f64` result, rejecting a non-finite value with a typed error
/// instead of letting it silently become null.
///
/// Mirrors the `**` operator's finiteness guard so the function and operator
/// forms agree.
fn finite_result(fn_name: &str, value: f64) -> ExpressionResult<RuntimeValue> {
    if value.is_finite() {
        Ok(RuntimeValue::Float(value))
    } else {
        Err(ExpressionError::invalid_argument(
            fn_name,
            "result is not a finite number",
        ))
    }
}

/// Read argument `index` as a numeric value, preserving its integer shape.
fn numeric_argument(
    function: &str,
    args: &[Argument<'_>],
    index: usize,
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    // Validate through the shared numeric coercion (strict-mode aware).
    get_number_arg_with_policy(function, args, index, "value", view, context)?;
    match get_value_arg(function, args, index, "value")? {
        RuntimeValue::Integer(_) | RuntimeValue::Unsigned(_) | RuntimeValue::Float(_) => {
            Ok(get_value_arg(function, args, index, "value")?.clone())
        },
        RuntimeValue::String(text) => crate::value_utils::parse_number(text)
            .map_err(|message| ExpressionError::invalid_argument(function, message)),
        RuntimeValue::Bool(value) => Ok(RuntimeValue::Integer(i64::from(*value))),
        _ => Err(ExpressionError::invalid_argument(
            function,
            "expected a finite number",
        )),
    }
}

/// Absolute value
pub(crate) fn abs(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("abs", args, 1)?;
    let number = numeric_argument("abs", args, 0, view, ctx)?;
    if let Some(integer) = number.as_i128() {
        return crate::value_utils::integer_result(integer.checked_abs(), "abs");
    }
    finite_result(
        "abs",
        get_number_arg_with_policy("abs", args, 0, "value", view, ctx)?.abs(),
    )
}

/// Round to specified decimal places (default: 0)
pub(crate) fn round(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("round", args, 1)?;
    let number = numeric_argument("round", args, 0, view, ctx)?;
    let decimals = if args.len() >= 2 {
        get_int_arg_with_policy("round", args, 1, "decimals", view, ctx)?
    } else {
        0
    };
    let decimals = i32::try_from(decimals)
        .ok()
        .filter(|value| (0..=308).contains(value))
        .ok_or_else(|| {
            ExpressionError::invalid_argument(
                "round",
                "decimals must be between 0 and 308 for a finite result",
            )
        })?;
    if !matches!(number, RuntimeValue::Float(_)) {
        return Ok(number);
    }
    let num = get_number_arg_with_policy("round", args, 0, "value", view, ctx)?;
    let multiplier = 10_f64.powi(decimals);
    finite_result("round", (num * multiplier).round() / multiplier)
}

/// Floor function
pub(crate) fn floor(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("floor", args, 1)?;
    let number = numeric_argument("floor", args, 0, view, ctx)?;
    if !matches!(number, RuntimeValue::Float(_)) {
        return Ok(number);
    }
    let num = get_number_arg_with_policy("floor", args, 0, "value", view, ctx)?;
    Ok(RuntimeValue::Float(num.floor()))
}

/// Ceiling function
pub(crate) fn ceil(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("ceil", args, 1)?;
    let number = numeric_argument("ceil", args, 0, view, ctx)?;
    if !matches!(number, RuntimeValue::Float(_)) {
        return Ok(number);
    }
    let num = get_number_arg_with_policy("ceil", args, 0, "value", view, ctx)?;
    Ok(RuntimeValue::Float(num.ceil()))
}

/// Minimum of two or more numbers
pub(crate) fn min(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("min", args, 1)?;

    extremum("min", args, view, ctx, std::cmp::Ordering::Less)
}

/// Maximum of two or more numbers
pub(crate) fn max(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("max", args, 1)?;

    extremum("max", args, view, ctx, std::cmp::Ordering::Greater)
}

fn extremum(
    function: &'static str,
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
    ordering: std::cmp::Ordering,
) -> ExpressionResult<RuntimeValue> {
    let mut best = numeric_argument(function, args, 0, view, context)?;
    for index in 1..args.len() {
        let candidate = numeric_argument(function, args, index, view, context)?;
        let comparison = crate::value_utils::compare_numbers(&candidate, &best).ok_or(
            ExpressionError::NonFiniteNumber {
                operation: function,
            },
        )?;
        if comparison == ordering {
            best = candidate;
        }
    }
    Ok(best)
}

/// Square root
pub(crate) fn sqrt(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("sqrt", args, 1)?;
    let num = get_number_arg_with_policy("sqrt", args, 0, "value", view, ctx)?;
    if num < 0.0 {
        return Err(ExpressionError::invalid_argument(
            "sqrt",
            "Cannot take square root of negative number",
        ));
    }
    Ok(RuntimeValue::Float(num.sqrt()))
}

/// Power function
pub(crate) fn pow(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("pow", args, 2)?;
    let base = get_number_arg_with_policy("pow", args, 0, "base", view, ctx)?;
    let exp = get_number_arg_with_policy("pow", args, 1, "exponent", view, ctx)?;
    // `powf` can overflow to `inf` (e.g. `pow(2, 1024)`) or be `NaN` (e.g.
    // `pow(-1, 0.5)`); reject both like the `**` operator does, rather than
    // emitting silent `null`.
    finite_result("pow", base.powf(exp))
}
