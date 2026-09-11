//! String manipulation functions

use serde_json::Value;

use super::{check_arg_count, check_min_arg_count, get_int_arg_with_policy, get_string_arg};
use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::{ExpressionErrorExt, ExpressionResult},
    eval::BuiltinView,
};

// Note: there used to be a `pub fn length` here that took a string only,
// duplicating the polymorphic `util::length` registered in
// `BuiltinRegistry::new()`. Removed in favor of the single polymorphic
// version in `util.rs`, which uses `value_utils::char_count` for
// strings (Unicode-scalar-value counting, NOT UTF-8 byte length and
// NOT JavaScript's UTF-16 code-unit count — see `char_count` docs).

fn preflight_string_output(
    view: BuiltinView<'_>,
    context: &EvaluationContext,
    output_bytes: usize,
) -> ExpressionResult<()> {
    view.check_output_bytes(output_bytes)?;
    let output = view.output_builder(context);
    output.ensure_string_bytes(output_bytes)?;
    output.ensure_total_bytes(output_bytes)
}

/// Convert string to uppercase
pub(crate) fn uppercase(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("uppercase", args, 1)?;
    let s = get_string_arg("uppercase", args, 0, "text")?;
    let output_bytes = s
        .chars()
        .flat_map(char::to_uppercase)
        .map(char::len_utf8)
        .fold(0usize, usize::saturating_add);
    preflight_string_output(view, context, output_bytes)?;
    Ok(Value::String(s.to_uppercase()))
}

/// Convert string to lowercase
pub(crate) fn lowercase(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("lowercase", args, 1)?;
    let s = get_string_arg("lowercase", args, 0, "text")?;
    let output_bytes = s
        .chars()
        .flat_map(char::to_lowercase)
        .map(char::len_utf8)
        .fold(0usize, usize::saturating_add);
    preflight_string_output(view, context, output_bytes)?;
    Ok(Value::String(s.to_lowercase()))
}

/// Trim whitespace from both ends of a string
pub(crate) fn trim(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("trim", args, 1)?;
    let s = get_string_arg("trim", args, 0, "text")?;
    let trimmed = s.trim();
    preflight_string_output(view, context, trimmed.len())?;
    Ok(Value::String(trimmed.to_owned()))
}

/// Split a string by a delimiter
pub(crate) fn split(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("split", args, 2)?;
    let s = get_string_arg("split", args, 0, "text")?;
    let delimiter = get_string_arg("split", args, 1, "delimiter")?;

    let mut part_count = 0usize;
    let mut total_bytes = 2usize;
    let mut max_part_bytes = 0usize;
    for part in s.split(delimiter) {
        part_count = part_count.saturating_add(1);
        max_part_bytes = max_part_bytes.max(part.len());
        total_bytes = total_bytes
            .saturating_add(usize::from(part_count > 1))
            .saturating_add(part.len());
    }
    view.check_output_bytes(total_bytes)?;
    let output = view.output_builder(context);
    output.ensure_collection_items(part_count)?;
    output.ensure_value_nodes(part_count.saturating_add(1))?;
    output.ensure_value_depth(2)?;
    output.ensure_string_bytes(max_part_bytes)?;
    output.ensure_total_bytes(total_bytes)?;

    let parts: Vec<_> = s
        .split(delimiter)
        .map(|s| Value::String(s.to_string()))
        .collect();
    Ok(Value::Array(parts))
}

/// Replace occurrences of a substring
pub(crate) fn replace(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("replace", args, 3)?;
    let s = get_string_arg("replace", args, 0, "text")?;
    let from = get_string_arg("replace", args, 1, "from")?;
    let to = get_string_arg("replace", args, 2, "to")?;

    let matches = s.matches(from).count();
    let output_bytes = s
        .len()
        .saturating_sub(matches.saturating_mul(from.len()))
        .saturating_add(matches.saturating_mul(to.len()));
    preflight_string_output(view, context, output_bytes)?;
    Ok(Value::String(s.replace(from, to)))
}

/// Extract a substring by Unicode scalar value indices.
///
/// Both `start` and `end` are character indices, NOT byte offsets — so
/// `substring("🙂hello", 0, 1)` returns `"🙂"`. When `end` is omitted it
/// defaults to the character count of the input. Out-of-range `end` is
/// clamped to the string's character length; `start > end` produces empty.
///
/// Note: indices are Rust scalar values, not JavaScript UTF-16 code
/// units; `substring("🙂", 0, 1)` returns the full emoji here, while JS
/// would return the high surrogate alone. See
/// `value_utils::char_count` for the rationale.
pub(crate) fn substring(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("substring", args, 2)?;
    let s = get_string_arg("substring", args, 0, "text")?;
    let start = get_int_arg_with_policy("substring", args, 1, "start", view, ctx)?;
    if start < 0 {
        return Err(ExpressionError::expression_invalid_argument(
            "substring",
            "Argument 'start' must be non-negative",
        ));
    }
    let start = start as usize;
    let character_count = s.chars().count();
    let end = if args.len() > 2 {
        let end = get_int_arg_with_policy("substring", args, 2, "end", view, ctx)?;
        if end < 0 {
            return Err(ExpressionError::expression_invalid_argument(
                "substring",
                "Argument 'end' must be non-negative",
            ));
        }
        (end as usize).min(character_count)
    } else {
        character_count
    };

    let start_byte = s
        .char_indices()
        .nth(start)
        .map_or(s.len(), |(offset, _)| offset);
    let end_byte = s
        .char_indices()
        .nth(end)
        .map_or(s.len(), |(offset, _)| offset);
    let selected = if start <= end {
        s.get(start_byte..end_byte).unwrap_or_default()
    } else {
        ""
    };
    preflight_string_output(view, ctx, selected.len())?;
    Ok(Value::String(selected.to_owned()))
}

/// Check if string contains a substring
pub(crate) fn contains(
    args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("contains", args, 2)?;
    let s = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[0]),
        )
    })?;
    let needle = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[1]),
        )
    })?;

    Ok(Value::Bool(s.contains(needle)))
}

/// Check if string starts with a prefix
pub(crate) fn starts_with(
    args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("starts_with", args, 2)?;
    let s = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[0]),
        )
    })?;
    let prefix = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[1]),
        )
    })?;

    Ok(Value::Bool(s.starts_with(prefix)))
}

/// Check if string ends with a suffix
pub(crate) fn ends_with(
    args: &[&Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("ends_with", args, 2)?;
    let s = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[0]),
        )
    })?;
    let suffix = args[1].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[1]),
        )
    })?;

    Ok(Value::Bool(s.ends_with(suffix)))
}

/// Pad a string from the left to a target length
///
/// Example: `pad_start("5", 3, "0")` returns `"005"`
/// Default fill character is a space.
pub(crate) fn pad_start(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("pad_start", args, 2)?;
    if args.len() > 3 {
        return Err(ExpressionError::expression_eval_error(format!(
            "pad_start: expected 2 or 3 arguments, got {}",
            args.len()
        )));
    }
    let s = get_string_arg("pad_start", args, 0, "text")?;
    let target_len = get_int_arg_with_policy("pad_start", args, 1, "length", view, ctx)?;
    if target_len < 0 {
        return Err(ExpressionError::expression_eval_error(
            "pad_start: length must be non-negative",
        ));
    }
    let target_len = target_len as usize;

    const MAX_PAD_LENGTH: usize = 1_048_576;
    if target_len > MAX_PAD_LENGTH {
        return Err(ExpressionError::expression_eval_error(format!(
            "pad_start: target length {target_len} exceeds maximum {MAX_PAD_LENGTH}"
        )));
    }

    let fill = if args.len() > 2 {
        get_string_arg("pad_start", args, 2, "fill_char")?
    } else {
        " "
    };
    if fill.is_empty() {
        return Err(ExpressionError::expression_invalid_argument(
            "pad_start",
            "Fill string must not be empty",
        ));
    }

    let char_count = s.chars().count();
    if char_count >= target_len {
        preflight_string_output(view, ctx, s.len())?;
        return Ok(Value::String(s.to_owned()));
    }

    let pad_len = target_len - char_count;
    preflight_string_output(
        view,
        ctx,
        s.len().saturating_add(padding_bytes(fill, pad_len)),
    )?;
    let padding: String = fill.chars().cycle().take(pad_len).collect();
    Ok(Value::String(format!("{padding}{s}")))
}

/// Pad a string from the right to a target length
///
/// Example: `pad_end("5", 3, "0")` returns `"500"`
/// Default fill character is a space.
pub(crate) fn pad_end(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_min_arg_count("pad_end", args, 2)?;
    if args.len() > 3 {
        return Err(ExpressionError::expression_eval_error(format!(
            "pad_end: expected 2 or 3 arguments, got {}",
            args.len()
        )));
    }
    let s = get_string_arg("pad_end", args, 0, "text")?;
    let target_len = get_int_arg_with_policy("pad_end", args, 1, "length", view, ctx)?;
    if target_len < 0 {
        return Err(ExpressionError::expression_eval_error(
            "pad_end: length must be non-negative",
        ));
    }
    let target_len = target_len as usize;

    const MAX_PAD_LENGTH: usize = 1_048_576;
    if target_len > MAX_PAD_LENGTH {
        return Err(ExpressionError::expression_eval_error(format!(
            "pad_end: target length {target_len} exceeds maximum {MAX_PAD_LENGTH}"
        )));
    }

    let fill = if args.len() > 2 {
        get_string_arg("pad_end", args, 2, "fill_char")?
    } else {
        " "
    };
    if fill.is_empty() {
        return Err(ExpressionError::expression_invalid_argument(
            "pad_end",
            "Fill string must not be empty",
        ));
    }

    let char_count = s.chars().count();
    if char_count >= target_len {
        preflight_string_output(view, ctx, s.len())?;
        return Ok(Value::String(s.to_owned()));
    }

    let pad_len = target_len - char_count;
    preflight_string_output(
        view,
        ctx,
        s.len().saturating_add(padding_bytes(fill, pad_len)),
    )?;
    let padding: String = fill.chars().cycle().take(pad_len).collect();
    Ok(Value::String(format!("{s}{padding}")))
}

/// Repeat a string N times
///
/// Example: `repeat("ab", 3)` returns `"ababab"`
pub(crate) fn repeat(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("repeat", args, 2)?;
    let s = get_string_arg("repeat", args, 0, "text")?;
    let count = get_int_arg_with_policy("repeat", args, 1, "count", view, ctx)?;
    if count < 0 {
        return Err(ExpressionError::expression_invalid_argument(
            "repeat",
            "Argument 'count' must be non-negative",
        ));
    }
    let count = count as usize;

    // Guard against excessive allocation
    const MAX_RESULT_LEN: usize = 1_000_000;
    let result_len = s.len().saturating_mul(count);
    if result_len > MAX_RESULT_LEN {
        return Err(ExpressionError::expression_eval_error(format!(
            "repeat would produce a string of {result_len} bytes, exceeding limit of {MAX_RESULT_LEN}"
        )));
    }

    preflight_string_output(view, ctx, result_len)?;
    Ok(Value::String(s.repeat(count)))
}

fn padding_bytes(fill: &str, characters: usize) -> usize {
    let fill_characters = fill.chars().count();
    let full = (characters / fill_characters).saturating_mul(fill.len());
    let tail: usize = fill
        .chars()
        .take(characters % fill_characters)
        .map(char::len_utf8)
        .sum();
    full.saturating_add(tail)
}
