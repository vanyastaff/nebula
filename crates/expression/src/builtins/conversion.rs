//! Type conversion functions

use std::io;

use crate::{
    ExpressionError,
    context::EvaluationContext,
    error::ExpressionResult,
    eval::{Argument, BuiltinView},
    value::RuntimeValue,
};

use super::{check_arg_count, get_value_arg, preflight_string_output};

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

fn measure_json(value: &RuntimeValue) -> ExpressionResult<usize> {
    let mut counter = JsonLength::default();
    serde_json::to_writer(&mut counter, &value.to_json()).map_err(|error| {
        ExpressionError::eval_error(format!("Failed to measure JSON output: {error}"))
    })?;
    Ok(counter.bytes)
}

fn encode_json(value: &RuntimeValue, output_bytes: usize) -> ExpressionResult<String> {
    let mut encoded = Vec::with_capacity(output_bytes);
    serde_json::to_writer(&mut encoded, &value.to_json()).map_err(|error| {
        ExpressionError::eval_error(format!("Failed to serialize to JSON: {error}"))
    })?;
    String::from_utf8(encoded).map_err(|error| {
        ExpressionError::eval_error(format!("JSON serialization was not UTF-8: {error}"))
    })
}

#[derive(Debug, Clone, Copy)]
enum JsonContainer {
    Array { expects_value: bool, items: usize },
    Object { items: usize },
}

/// Count one entry of the innermost container and preflight the output
/// limits it consumes: a value in value position inside an array (a scalar,
/// string, or nested opener), or an object key at its `:`. The array's
/// closing bracket and comma keep it waiting for its next entry, so neither
/// counts.
fn count_container_entry(
    containers: &mut [JsonContainer],
    nodes: &mut usize,
    output: crate::BuiltinOutputBuilder,
    byte: u8,
) -> ExpressionResult<()> {
    let Some(innermost) = containers.last_mut() else {
        return Ok(());
    };
    let items = match innermost {
        JsonContainer::Array {
            expects_value,
            items,
        } if *expects_value && byte != b']' && byte != b',' => {
            *expects_value = false;
            items
        },
        JsonContainer::Object { items } if byte == b':' => items,
        _ => return Ok(()),
    };
    *items = items.saturating_add(1);
    *nodes = nodes.saturating_add(1);
    output.ensure_collection_items(*items)?;
    output.ensure_value_nodes(*nodes)?;
    output.ensure_value_depth(containers.len().saturating_add(1))
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

        count_container_entry(&mut containers, &mut nodes, output, byte)?;

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
            _ => {},
        }
    }
    Ok(())
}

/// Convert value to string
pub(crate) fn to_string(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("to_string", args, 1)?;
    let value = get_value_arg("to_string", args, 0, "value")?;

    if view.strict_conversions_enabled()
        && matches!(value, RuntimeValue::Array(_) | RuntimeValue::Object(_))
    {
        return Err(ExpressionError::type_error(
            "scalar (string/number/boolean/null)",
            crate::value_utils::value_type_name(value),
        ));
    }

    let output_bytes = match value {
        RuntimeValue::String(text) => text.len(),
        RuntimeValue::Array(_) | RuntimeValue::Object(_) => measure_json(value)?,
        _ => value.to_json().to_string().len(),
    };
    preflight_string_output(view, output_bytes)?;
    Ok(RuntimeValue::string(value.to_display_string()))
}

/// Convert value to number
pub(crate) fn to_number(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("to_number", args, 1)?;
    let value = get_value_arg("to_number", args, 0, "value")?;

    if view.strict_conversions_enabled() && !value.is_number() {
        return Err(ExpressionError::type_error(
            "number",
            crate::value_utils::value_type_name(value),
        ));
    }

    match value {
        RuntimeValue::Integer(_) | RuntimeValue::Unsigned(_) | RuntimeValue::Float(_) => {
            Ok(value.clone())
        },
        RuntimeValue::Bool(value) => Ok(RuntimeValue::Integer(i64::from(*value))),
        RuntimeValue::String(text) => crate::value_utils::parse_number(text)
            .map_err(|message| ExpressionError::invalid_argument("to_number", message)),
        other => Err(ExpressionError::type_error(
            "convertible to number",
            crate::value_utils::value_type_name(other),
        )),
    }
}

/// Convert value to boolean
pub(crate) fn to_boolean(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("to_boolean", args, 1)?;
    let value = get_value_arg("to_boolean", args, 0, "value")?;

    if view.strict_conversions_enabled() && value.as_bool().is_none() {
        return Err(ExpressionError::type_error(
            "boolean",
            crate::value_utils::value_type_name(value),
        ));
    }

    Ok(RuntimeValue::Bool(crate::value_utils::to_boolean(value)))
}

/// Convert value to JSON string
pub(crate) fn to_json(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _context: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("to_json", args, 1)?;
    let value = get_value_arg("to_json", args, 0, "value")?;

    let output_bytes = measure_json(value)?;
    preflight_string_output(view, output_bytes)?;
    let json_string = encode_json(value, output_bytes)?;

    Ok(RuntimeValue::string(json_string))
}

/// Parse JSON string to value
pub(crate) fn parse_json(
    args: &[Argument<'_>],
    view: BuiltinView<'_>,
    _ctx: &EvaluationContext,
) -> ExpressionResult<RuntimeValue> {
    check_arg_count("parse_json", args, 1)?;
    let value = get_value_arg("parse_json", args, 0, "value")?;
    let json_str = value.as_str().ok_or_else(|| {
        ExpressionError::type_error("string", crate::value_utils::value_type_name(value))
    })?;

    // DoS protection: limit JSON string size. The effective policy already
    // resolved the engine default into the frame, so no fallback is needed.
    let max_len = view.max_json_parse_length();
    if json_str.len() > max_len {
        return Err(ExpressionError::eval_error(format!(
            "JSON string too large: {} bytes (max {} bytes)",
            json_str.len(),
            max_len
        )));
    }

    let output = view.output_builder();
    preflight_json_structure(json_str, output)?;

    let json: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| ExpressionError::invalid_json(format!("failed to parse JSON: {e}")))?;
    let json = RuntimeValue::from_json(&json);

    if view.strict_conversions_enabled()
        && !matches!(json, RuntimeValue::Object(_) | RuntimeValue::Array(_))
    {
        return Err(ExpressionError::type_error(
            "object or array",
            crate::value_utils::value_type_name(&json),
        ));
    }

    output.value(json).map(crate::BuiltinOutput::into_value)
}
