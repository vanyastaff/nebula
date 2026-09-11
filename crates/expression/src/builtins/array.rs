//! Array manipulation functions

use std::fmt::{self, Write as _};

use serde_json::Value;

use super::{check_arg_count, check_min_arg_count, get_array_arg};
use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::{ExpressionErrorExt, ExpressionResult},
    eval::BuiltinView,
};

// Note: there used to be a `pub fn length` here that took an array only,
// duplicating the polymorphic `util::length` registered in
// `BuiltinRegistry::new()`. It was unused (the registry never wired it up)
// and was a maintenance hazard — the array-only and string-only copies
// could drift from the polymorphic version. Removed; `util::length`
// handles strings, arrays, and objects in one place.

/// Get the first element of an array
pub(crate) fn first(
    args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("first", args, 1)?;
    let arr = get_array_arg("first", args, 0, "array")?;
    let json_val = arr
        .first()
        .ok_or_else(|| ExpressionError::expression_eval_error("Array is empty"))?;
    Ok(json_val.clone())
}

/// Get the last element of an array
pub(crate) fn last(
    args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("last", args, 1)?;
    let arr = get_array_arg("last", args, 0, "array")?;
    let len = arr.len();
    if len == 0 {
        return Err(ExpressionError::expression_eval_error("Array is empty"));
    }
    let json_val = arr
        .get(len - 1)
        .ok_or_else(|| ExpressionError::expression_eval_error("Array is empty"))?;
    Ok(json_val.clone())
}

/// Filter array elements (stub - lambdas need special handling)
pub(crate) fn filter(
    _args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    // Note: This would require special handling in the evaluator to pass lambdas
    Err(ExpressionError::expression_eval_error(
        "filter requires lambda support in evaluator",
    ))
}

/// Map over array elements (stub - lambdas need special handling)
pub(crate) fn map(
    _args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(ExpressionError::expression_eval_error(
        "map requires lambda support in evaluator",
    ))
}

/// Reduce array elements (stub - lambdas need special handling)
pub(crate) fn reduce(
    _args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    Err(ExpressionError::expression_eval_error(
        "reduce requires lambda support in evaluator",
    ))
}

/// Sort an array
pub(crate) fn sort(
    args: &[&Value],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("sort", args, 1)?;
    let arr = get_array_arg("sort", args, 0, "array")?;

    // Stable sorting performs O(n log n) comparisons in the worst case.
    // String comparison can inspect every byte in the longest operand, so
    // charge that upper bound before cloning or sorting the array.
    let comparison_count = arr.len().saturating_mul(arr.len().bit_width() as usize);
    let comparison_bytes = arr
        .iter()
        .filter_map(Value::as_str)
        .map(str::len)
        .max()
        .unwrap_or(1);
    view.charge_work(comparison_count.saturating_mul(comparison_bytes))?;

    let mut elements: Vec<Value> = arr.clone();

    // Sort the values
    elements.sort_by(|a, b| match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            crate::value_utils::compare_numbers(x, y).unwrap_or(std::cmp::Ordering::Equal)
        },
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    });

    Ok(Value::Array(elements))
}

/// Reverse an array
pub(crate) fn reverse(
    args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("reverse", args, 1)?;
    let arr = get_array_arg("reverse", args, 0, "array")?;

    let mut elements: Vec<Value> = arr.clone();
    elements.reverse();

    Ok(Value::Array(elements))
}

/// Join array elements into a string
pub(crate) fn join(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("join", args, 2)?;
    let arr = get_array_arg("join", args, 0, "array")?;
    let separator = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[1]),
        )
    })?;

    let mut output_bytes = separator.len().saturating_mul(arr.len().saturating_sub(1));
    for value in arr {
        let element_bytes = match value {
            Value::String(string) => string.len(),
            value => {
                let mut counter = FormatLength::default();
                write!(&mut counter, "{value}").map_err(|_| {
                    ExpressionError::expression_eval_error("failed to measure join output")
                })?;
                counter.bytes
            },
        };
        output_bytes = output_bytes.saturating_add(element_bytes);
    }
    view.check_output_bytes(output_bytes)?;
    let output = view.output_builder(context);
    output.ensure_string_bytes(output_bytes)?;
    output.ensure_total_bytes(output_bytes)?;

    let mut result = String::with_capacity(output_bytes);
    for (index, value) in arr.iter().enumerate() {
        if index > 0 {
            result.push_str(separator);
        }
        match value {
            Value::String(string) => result.push_str(string),
            value => write!(&mut result, "{value}").map_err(|_| {
                ExpressionError::expression_eval_error("failed to render join output")
            })?,
        }
    }

    Ok(Value::String(result))
}

#[derive(Default)]
struct FormatLength {
    bytes: usize,
}

impl fmt::Write for FormatLength {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.bytes = self.bytes.saturating_add(value.len());
        Ok(())
    }
}

/// Resolve a slice bound: a negative index counts from the end (like `arr[-1]`),
/// then the result is clamped to `[0, len]` (slice semantics never go
/// out of bounds — unlike indexing, which errors).
fn resolve_slice_bound(index: i64, len: usize) -> usize {
    let len_i64 = len as i64;
    let resolved = if index < 0 { len_i64 + index } else { index };
    resolved.clamp(0, len_i64) as usize
}

/// Slice an array
pub(crate) fn slice(
    args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("slice", args, 2)?;
    let arr = get_array_arg("slice", args, 0, "array")?;
    let start_index = args[1].as_i64().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "integer",
            crate::value_utils::value_type_name(args[1]),
        )
    })?;
    let end_index = if args.len() > 2 {
        args[2].as_i64().ok_or_else(|| {
            ExpressionError::expression_type_error(
                "integer",
                crate::value_utils::value_type_name(args[2]),
            )
        })?
    } else {
        arr.len() as i64
    };

    // Negative bounds count from the end and match the `arr[-1]` index operator;
    // both bounds clamp to the array, so an out-of-range slice is empty, never a
    // panic. `get(start..end)` is `None` when `start > end`, also yielding empty.
    let start = resolve_slice_bound(start_index, arr.len());
    let end = resolve_slice_bound(end_index, arr.len());
    let result = arr
        .get(start..end)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    Ok(Value::Array(result))
}

/// Concatenate arrays
pub(crate) fn concat(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("concat", args, 1)?;

    let mut total_size = 0usize;
    for (index, _) in args.iter().enumerate() {
        let array = get_array_arg("concat", args, index, "array")?;
        total_size = total_size.saturating_add(array.len());
    }
    view.output_builder(context).preflight_array(
        args.iter()
            .filter_map(|argument| argument.as_array())
            .flatten(),
    )?;

    let mut result = Vec::with_capacity(total_size);
    for (i, _arg) in args.iter().enumerate() {
        let arr = get_array_arg("concat", args, i, "array")?;
        result.extend(arr.iter().cloned());
    }

    Ok(Value::Array(result))
}

/// Remove duplicate elements from an array, preserving order
///
/// Uses string representation for equality comparison.
/// Example: `unique([1,2,2,3,1])` returns `[1,2,3]`
pub(crate) fn unique(
    args: &[&Value],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("unique", args, 1)?;
    let arr = get_array_arg("unique", args, 0, "array")?;

    let mut scratch_bytes = 0usize;
    for item in arr {
        let mut counter = FormatLength::default();
        write!(&mut counter, "{item}").map_err(|_| {
            ExpressionError::expression_eval_error("failed to measure unique comparison key")
        })?;
        scratch_bytes = scratch_bytes.saturating_add(counter.bytes);
        crate::limits::check_limit(
            "builtin scratch bytes",
            scratch_bytes,
            crate::limits::MAX_RESULT_BYTES,
        )?;
    }
    view.charge_work(scratch_bytes)?;

    let mut seen = std::collections::HashSet::new();
    let result: Vec<Value> = arr
        .iter()
        .filter(|item| {
            // Use JSON serialization for stable equality comparison
            let key = item.to_string();
            seen.insert(key)
        })
        .cloned()
        .collect();

    Ok(Value::Array(result))
}

/// Flatten a nested array
pub(crate) fn flatten(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("flatten", args, 1)?;
    let arr = get_array_arg("flatten", args, 0, "array")?;

    view.output_builder(context)
        .preflight_array(arr.iter().flat_map(|value| {
            value
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| std::slice::from_ref(value))
        }))?;

    let result: Vec<_> = arr
        .iter()
        .flat_map(|elem| {
            if let Some(inner_arr) = elem.as_array() {
                inner_arr.clone()
            } else {
                vec![elem.clone()]
            }
        })
        .collect();

    Ok(Value::Array(result))
}

// Note: some, every, find, find_index, group_by, flat_map are higher-order
// functions implemented in the evaluator (eval.rs). They require lambda
// arguments and are dispatched via try_higher_order_function before reaching
// the builtin registry.
