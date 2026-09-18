//! Array manipulation functions

use std::{fmt, fmt::Write as _};

use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

use super::{check_arg_count, check_min_arg_count, get_array_arg, get_value_arg};

/// Get the first element of an array
pub(crate) fn first(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("first", args, 1)?;
    let array = get_array_arg("first", args, 0, "array")?;
    array
        .first()
        .cloned()
        .ok_or_else(|| ExpressionError::eval_error("Array is empty"))
}

/// Get the last element of an array
pub(crate) fn last(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("last", args, 1)?;
    let array = get_array_arg("last", args, 0, "array")?;
    array
        .last()
        .cloned()
        .ok_or_else(|| ExpressionError::eval_error("Array is empty"))
}

/// Sort an array
pub(crate) fn sort(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("sort", args, 1)?;
    let array = get_array_arg("sort", args, 0, "array")?;

    // Stable sorting performs O(n log n) comparisons in the worst case.
    // String comparison can inspect every byte in the longest operand, so
    // charge that upper bound before cloning or sorting the array.
    let comparison_count = array.len().saturating_mul(array.len().bit_width() as usize);
    let comparison_bytes = array
        .iter()
        .filter_map(RuntimeValue::as_str)
        .map(str::len)
        .max()
        .unwrap_or(1);
    view.charge_work(comparison_count.saturating_mul(comparison_bytes))?;

    let mut elements = array.to_vec();

    // Sort the values
    elements.sort_by(|a, b| match (a, b) {
        (RuntimeValue::String(x), RuntimeValue::String(y)) => x.cmp(y),
        _ if a.is_number() && b.is_number() => {
            crate::value_utils::compare_numbers(a, b).unwrap_or(std::cmp::Ordering::Equal)
        },
        _ => std::cmp::Ordering::Equal,
    });

    Ok(RuntimeValue::Array(elements.into()))
}

/// Reverse an array
pub(crate) fn reverse(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("reverse", args, 1)?;
    let array = get_array_arg("reverse", args, 0, "array")?;

    let mut elements = array.to_vec();
    elements.reverse();

    Ok(RuntimeValue::Array(elements.into()))
}

/// First index of a value in an array, or `-1`.
///
/// Uses structural equality (`RuntimeValue::eq`), so numbers compare exactly
/// across representations and objects/arrays compare by contents.
pub(crate) fn index_of(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("index_of", args, 2)?;
    let array = get_array_arg("index_of", args, 0, "array")?;
    let needle = get_value_arg("index_of", args, 1, "value")?;
    for (index, item) in array.iter().enumerate() {
        if item == needle {
            return Ok(RuntimeValue::Integer(index as i64));
        }
    }
    Ok(RuntimeValue::Integer(-1))
}

/// Join array elements into a string
pub(crate) fn join(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("join", args, 2)?;
    let array = get_array_arg("join", args, 0, "array")?;
    let separator_value = get_value_arg("join", args, 1, "separator")?;
    let separator = separator_value.as_str().ok_or_else(|| {
        ExpressionError::type_error(
            "string",
            crate::value_utils::value_type_name(separator_value),
        )
    })?;

    let mut output_bytes = separator
        .len()
        .saturating_mul(array.len().saturating_sub(1));
    for value in array {
        let element_bytes = match value {
            RuntimeValue::String(text) => text.len(),
            value => {
                let mut counter = FormatLength::default();
                write!(&mut counter, "{}", value.to_json())
                    .map_err(|_| ExpressionError::eval_error("failed to measure join output"))?;
                counter.bytes
            },
        };
        output_bytes = output_bytes.saturating_add(element_bytes);
    }
    view.check_output_bytes(output_bytes)?;
    let output = view.output_builder();
    output.ensure_string_bytes(output_bytes)?;
    output.ensure_total_bytes(output_bytes)?;

    let mut result = String::with_capacity(output_bytes);
    for (index, value) in array.iter().enumerate() {
        if index > 0 {
            result.push_str(separator);
        }
        match value {
            RuntimeValue::String(text) => result.push_str(text),
            value => write!(&mut result, "{}", value.to_json())
                .map_err(|_| ExpressionError::eval_error("failed to render join output"))?,
        }
    }

    Ok(RuntimeValue::string(result))
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
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("slice", args, 2)?;
    let array = get_array_arg("slice", args, 0, "array")?;
    let start_value = get_value_arg("slice", args, 1, "start")?;
    let start_index = start_value.as_i64().ok_or_else(|| {
        ExpressionError::type_error("integer", crate::value_utils::value_type_name(start_value))
    })?;
    let end_index = if args.len() > 2 {
        let end_value = get_value_arg("slice", args, 2, "end")?;
        end_value.as_i64().ok_or_else(|| {
            ExpressionError::type_error("integer", crate::value_utils::value_type_name(end_value))
        })?
    } else {
        array.len() as i64
    };

    // Negative bounds count from the end and match the `arr[-1]` index operator;
    // both bounds clamp to the array, so an out-of-range slice is empty, never a
    // panic. `get(start..end)` is `None` when `start > end`, also yielding empty.
    let start = resolve_slice_bound(start_index, array.len());
    let end = resolve_slice_bound(end_index, array.len());
    let result = array
        .get(start..end)
        .map(<[RuntimeValue]>::to_vec)
        .unwrap_or_default();
    Ok(RuntimeValue::Array(result.into()))
}

/// Concatenate arrays
pub(crate) fn concat(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("concat", args, 1)?;

    let mut total_size = 0usize;
    for (index, _) in args.iter().enumerate() {
        let array = get_array_arg("concat", args, index, "array")?;
        total_size = total_size.saturating_add(array.len());
    }
    view.output_builder().preflight_array(
        args.iter()
            .filter_map(Argument::as_value)
            .filter_map(RuntimeValue::as_array)
            .flatten(),
    )?;

    let mut result = Vec::with_capacity(total_size);
    for (index, _) in args.iter().enumerate() {
        let array = get_array_arg("concat", args, index, "array")?;
        result.extend(array.iter().cloned());
    }

    Ok(RuntimeValue::Array(result.into()))
}

/// Remove duplicate elements from an array, preserving order
///
/// Uses JSON representation for equality comparison.
/// Example: `unique([1,2,2,3,1])` returns `[1,2,3]`
pub(crate) fn unique(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("unique", args, 1)?;
    let array = get_array_arg("unique", args, 0, "array")?;

    let mut scratch_bytes = 0usize;
    for item in array {
        let mut counter = FormatLength::default();
        write!(&mut counter, "{}", item.to_json())
            .map_err(|_| ExpressionError::eval_error("failed to measure unique comparison key"))?;
        scratch_bytes = scratch_bytes.saturating_add(counter.bytes);
        crate::limits::check_limit(
            "builtin scratch bytes",
            scratch_bytes,
            crate::limits::MAX_RESULT_BYTES,
        )?;
    }
    view.charge_work(scratch_bytes)?;

    let mut seen = std::collections::HashSet::new();
    let result: Vec<RuntimeValue> = array
        .iter()
        .filter(|item| {
            // Use JSON serialization for stable equality comparison
            let key = item.to_json().to_string();
            seen.insert(key)
        })
        .cloned()
        .collect();

    Ok(RuntimeValue::Array(result.into()))
}

/// Flatten a nested array
pub(crate) fn flatten(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("flatten", args, 1)?;
    let array = get_array_arg("flatten", args, 0, "array")?;

    view.output_builder()
        .preflight_array(array.iter().flat_map(|value| {
            value
                .as_array()
                .unwrap_or_else(|| std::slice::from_ref(value))
        }))?;

    let result: Vec<RuntimeValue> = array
        .iter()
        .flat_map(|element| {
            if let Some(inner) = element.as_array() {
                inner.to_vec()
            } else {
                vec![element.clone()]
            }
        })
        .collect();

    Ok(RuntimeValue::Array(result.into()))
}
