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
// version in `util.rs`, which already uses `value_utils::char_count` for
// strings (n8n-compatible code-unit counting, not UTF-8 byte length).

/// Convert string to uppercase
pub fn uppercase(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("uppercase", args, 1)?;
    let s = get_string_arg("uppercase", args, 0, "text")?;
    Ok(Value::String(s.to_uppercase()))
}

/// Convert string to lowercase
pub fn lowercase(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("lowercase", args, 1)?;
    let s = get_string_arg("lowercase", args, 0, "text")?;
    Ok(Value::String(s.to_lowercase()))
}

/// Trim whitespace from both ends of a string
pub fn trim(
    args: &[Value],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("trim", args, 1)?;
    let s = get_string_arg("trim", args, 0, "text")?;
    Ok(Value::String(s.trim().to_string()))
}

/// Split a string by a delimiter
pub fn split(
    args: &[Value],
    _view: BuiltinView<'_>,
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
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("replace", args, 3)?;
    let s = get_string_arg("replace", args, 0, "text")?;
    let from = get_string_arg("replace", args, 1, "from")?;
    let to = get_string_arg("replace", args, 2, "to")?;

    Ok(Value::String(s.replace(from, to)))
}

/// Extract a substring by Unicode scalar value indices (n8n-compatible).
///
/// Both `start` and `end` are character indices, NOT byte offsets — so
/// `substring("🙂hello", 0, 1)` returns `"🙂"`. When `end` is omitted it
/// defaults to the character count of the input. Out-of-range `end` is
/// clamped to the string's character length; `start > end` produces empty.
pub fn substring(
    args: &[Value],
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
    let chars: Vec<char> = s.chars().collect();
    let start = start as usize;
    let end = if args.len() > 2 {
        let end = get_int_arg_with_policy("substring", args, 2, "end", view, ctx)?;
        if end < 0 {
            return Err(ExpressionError::expression_invalid_argument(
                "substring",
                "Argument 'end' must be non-negative",
            ));
        }
        (end as usize).min(chars.len())
    } else {
        chars.len()
    };

    let result: String = chars.get(start..end).unwrap_or(&[]).iter().collect();
    Ok(Value::String(result))
}

/// Check if string contains a substring
pub fn contains(
    args: &[Value],
    _view: BuiltinView<'_>,
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
    _view: BuiltinView<'_>,
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
    _view: BuiltinView<'_>,
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

/// Pad a string from the left to a target length
///
/// Example: `pad_start("5", 3, "0")` returns `"005"`
/// Default fill character is a space.
pub fn pad_start(
    args: &[Value],
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
        return Ok(Value::String(s.to_string()));
    }

    let pad_len = target_len - char_count;
    let padding: String = fill.chars().cycle().take(pad_len).collect();
    Ok(Value::String(format!("{padding}{s}")))
}

/// Pad a string from the right to a target length
///
/// Example: `pad_end("5", 3, "0")` returns `"500"`
/// Default fill character is a space.
pub fn pad_end(
    args: &[Value],
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
        return Ok(Value::String(s.to_string()));
    }

    let pad_len = target_len - char_count;
    let padding: String = fill.chars().cycle().take(pad_len).collect();
    Ok(Value::String(format!("{s}{padding}")))
}

/// Repeat a string N times
///
/// Example: `repeat("ab", 3)` returns `"ababab"`
pub fn repeat(
    args: &[Value],
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

    Ok(Value::String(s.repeat(count)))
}
