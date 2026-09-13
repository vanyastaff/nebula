//! Type conversion functions

use std::io;

use serde_json::Value;

use super::check_arg_count;
use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::{ExpressionErrorExt, ExpressionResult},
    eval::BuiltinView,
};

/// Maximum JSON string length to parse (1MB) - DoS protection
const MAX_JSON_PARSE_LENGTH: usize = 1024 * 1024;

#[derive(Default)]
struct JsonLength {
    bytes: usize,
}

impl io::Write for JsonLength {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn measure_json(value: &Value) -> ExpressionResult<usize> {
    let mut counter = JsonLength::default();
    serde_json::to_writer(&mut counter, value).map_err(|error| {
        ExpressionError::expression_eval_error(format!("Failed to measure JSON output: {error}"))
    })?;
    Ok(counter.bytes)
}

fn encode_json(value: &Value, output_bytes: usize) -> ExpressionResult<String> {
    let mut encoded = Vec::with_capacity(output_bytes);
    serde_json::to_writer(&mut encoded, value).map_err(|error| {
        ExpressionError::expression_eval_error(format!("Failed to serialize to JSON: {error}"))
    })?;
    String::from_utf8(encoded).map_err(|error| {
        ExpressionError::expression_eval_error(format!("JSON serialization was not UTF-8: {error}"))
    })
}

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

#[derive(Debug, Clone, Copy)]
enum JsonContainer {
    Array { expects_value: bool, items: usize },
    Object { items: usize },
}

fn preflight_json_structure(
    source: &str,
    output: crate::BuiltinOutputBuilder,
) -> ExpressionResult<()> {
    if source.trim().is_empty() {
        return Ok(());
    }

    let mut containers = Vec::new();
    let mut nodes = 1usize;
    let mut in_string = false;
    let mut escaped = false;
    output.ensure_value_nodes(nodes)?;
    output.ensure_value_depth(1)?;

    for byte in source.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }

        if byte.is_ascii_whitespace() {
            continue;
        }

        if let Some(JsonContainer::Array {
            expects_value,
            items,
        }) = containers.last_mut()
            && *expects_value
            && byte != b']'
            && byte != b','
        {
            *expects_value = false;
            *items = items.saturating_add(1);
            nodes = nodes.saturating_add(1);
            output.ensure_collection_items(*items)?;
            output.ensure_value_nodes(nodes)?;
            output.ensure_value_depth(containers.len().saturating_add(1))?;
        }

        match byte {
            b'"' => in_string = true,
            b'[' => {
                containers.push(JsonContainer::Array {
                    expects_value: true,
                    items: 0,
                });
                output.ensure_value_depth(containers.len())?;
            },
            b'{' => {
                containers.push(JsonContainer::Object { items: 0 });
                output.ensure_value_depth(containers.len())?;
            },
            b']' | b'}' => {
                containers.pop();
            },
            b',' => {
                if let Some(JsonContainer::Array { expects_value, .. }) = containers.last_mut() {
                    *expects_value = true;
                }
            },
            b':' => {
                if let Some(JsonContainer::Object { items }) = containers.last_mut() {
                    *items = items.saturating_add(1);
                    nodes = nodes.saturating_add(1);
                    output.ensure_collection_items(*items)?;
                    output.ensure_value_nodes(nodes)?;
                    output.ensure_value_depth(containers.len().saturating_add(1))?;
                }
            },
            _ => {},
        }
    }
    Ok(())
}

/// Convert value to string
pub(crate) fn to_string(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_string", args, 1)?;

    if view.strict_conversions_enabled(ctx) && matches!(args[0], Value::Array(_) | Value::Object(_))
    {
        return Err(ExpressionError::expression_type_error(
            "scalar (string/number/boolean/null)",
            crate::value_utils::value_type_name(args[0]),
        ));
    }

    let output_bytes = match args[0] {
        Value::String(string) => string.len(),
        Value::Number(number) => measure_json(&Value::Number(number.clone()))?,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Null => 4,
        Value::Array(_) | Value::Object(_) => measure_json(args[0])?,
    };
    preflight_string_output(view, ctx, output_bytes)?;
    let string_val = match args[0] {
        Value::String(string) => string.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_owned(),
        Value::Array(_) | Value::Object(_) => encode_json(args[0], output_bytes)?,
    };
    Ok(Value::String(string_val))
}

/// Convert value to number
pub(crate) fn to_number(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_number", args, 1)?;

    if view.strict_conversions_enabled(ctx) && !args[0].is_number() {
        return Err(ExpressionError::expression_type_error(
            "number",
            crate::value_utils::value_type_name(args[0]),
        ));
    }

    match args[0] {
        Value::Number(number) => Ok(Value::Number(number.clone())),
        Value::Bool(value) => Ok(Value::from(i64::from(*value))),
        Value::String(text) => crate::value_utils::parse_number(text)
            .map(Value::Number)
            .map_err(|message| ExpressionError::invalid_argument("to_number", message)),
        other => Err(ExpressionError::type_error(
            "convertible to number",
            crate::value_utils::value_type_name(other),
        )),
    }
}

/// Convert value to boolean
pub(crate) fn to_boolean(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_boolean", args, 1)?;

    if view.strict_conversions_enabled(ctx) && !args[0].is_boolean() {
        return Err(ExpressionError::expression_type_error(
            "boolean",
            crate::value_utils::value_type_name(args[0]),
        ));
    }

    Ok(Value::Bool(crate::value_utils::to_boolean(args[0])))
}

/// Convert value to JSON string
pub(crate) fn to_json(
    args: &[&Value],
    view: BuiltinView<'_>,
    context: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("to_json", args, 1)?;

    let output_bytes = measure_json(args[0])?;
    preflight_string_output(view, context, output_bytes)?;
    let json_string = encode_json(args[0], output_bytes)?;

    Ok(Value::String(json_string))
}

/// Parse JSON string to value
pub(crate) fn parse_json(
    args: &[&Value],
    view: BuiltinView<'_>,
    ctx: &EvaluationContext,
) -> ExpressionResult<Value> {
    check_arg_count("parse_json", args, 1)?;

    let json_str = args[0].as_str().ok_or_else(|| {
        ExpressionError::expression_type_error(
            "string",
            crate::value_utils::value_type_name(args[0]),
        )
    })?;

    // DoS protection: limit JSON string size
    let max_len = view
        .max_json_parse_length(ctx)
        .unwrap_or(MAX_JSON_PARSE_LENGTH);
    if json_str.len() > max_len {
        return Err(ExpressionError::expression_eval_error(format!(
            "JSON string too large: {} bytes (max {} bytes)",
            json_str.len(),
            max_len
        )));
    }

    let output = view.output_builder(ctx);
    preflight_json_structure(json_str, output)?;

    let json: Value = serde_json::from_str(json_str).map_err(|e| {
        ExpressionError::expression_eval_error(format!("Failed to parse JSON: {e}"))
    })?;

    if view.strict_conversions_enabled(ctx) && !matches!(json, Value::Object(_) | Value::Array(_)) {
        return Err(ExpressionError::expression_type_error(
            "object or array",
            crate::value_utils::value_type_name(&json),
        ));
    }

    output.value(json).map(crate::BuiltinOutput::into_value)
}
