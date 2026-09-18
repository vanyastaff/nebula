//! Utility functions

use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

use super::{check_arg_count, check_min_arg_count, get_value_arg};

/// Get the length of a string, array, or object — the single polymorphic
/// `length()` exposed to expressions.
///
/// - **String**: Unicode scalar values (`chars`). `length("🙂")` is 1, `length("über")` is 4. This
///   deviates from JavaScript / n8n's UTF-16 code-unit semantics (which would return 2 for `"🙂"`)
///   — see `value_utils::char_count` for the rationale.
/// - **Array**: number of elements.
/// - **Object**: number of top-level keys.
///
/// All other input types yield a typed error.
pub(crate) fn length(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("length", args, 1)?;
    let value = get_value_arg("length", args, 0, "value")?;
    match value {
        RuntimeValue::String(text) => {
            Ok(RuntimeValue::Integer(crate::value_utils::char_count(text)))
        },
        RuntimeValue::Array(values) => Ok(RuntimeValue::Integer(values.len() as i64)),
        RuntimeValue::Object(entries) => Ok(RuntimeValue::Integer(entries.len() as i64)),
        _ => Err(ExpressionError::type_error(
            "string, array, or object",
            crate::value_utils::value_type_name(value),
        )),
    }
}

/// Check if value is null
pub(crate) fn is_null(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("is_null", args, 1)?;
    let value = get_value_arg("is_null", args, 0, "value")?;
    Ok(RuntimeValue::Bool(value.is_nullish()))
}

/// Check if value is an array
pub(crate) fn is_array(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("is_array", args, 1)?;
    Ok(RuntimeValue::Bool(
        get_value_arg("is_array", args, 0, "value")?
            .as_array()
            .is_some(),
    ))
}

/// Check if value is an object
pub(crate) fn is_object(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("is_object", args, 1)?;
    Ok(RuntimeValue::Bool(
        get_value_arg("is_object", args, 0, "value")?
            .as_object()
            .is_some(),
    ))
}

/// Check if value is a string
pub(crate) fn is_string(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("is_string", args, 1)?;
    Ok(RuntimeValue::Bool(
        get_value_arg("is_string", args, 0, "value")?
            .as_str()
            .is_some(),
    ))
}

/// Check if value is a number
pub(crate) fn is_number(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("is_number", args, 1)?;
    Ok(RuntimeValue::Bool(
        get_value_arg("is_number", args, 0, "value")?.is_number(),
    ))
}

/// Generate a new UUID
#[cfg(feature = "uuid")]
pub(crate) fn uuid(
    _args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    let id = uuid::Uuid::new_v4();
    Ok(RuntimeValue::string(id.to_string()))
}

/// Generate a new UUID (fallback when feature disabled)
#[cfg(not(feature = "uuid"))]
pub(crate) fn uuid(
    _args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    Err(ExpressionError::function_not_found(
        "uuid (feature 'uuid' not enabled)",
    ))
}

/// Return the first non-null value from the arguments
///
/// Example: `coalesce(null, null, 42, "hello")` returns `42`
pub(crate) fn coalesce(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_min_arg_count("coalesce", args, 1)?;

    for (index, _) in args.iter().enumerate() {
        let value = get_value_arg("coalesce", args, index, "value")?;
        if !value.is_nullish() {
            return Ok(value.clone());
        }
    }

    Ok(RuntimeValue::Null)
}

/// Return the type name of a value as a string
///
/// Example: `type_of(42)` returns `"number"`
pub(crate) fn type_of(
    args: &[Argument<'_>],
    _view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("type_of", args, 1)?;
    Ok(RuntimeValue::string(crate::value_utils::value_type_name(
        get_value_arg("type_of", args, 0, "value")?,
    )))
}
