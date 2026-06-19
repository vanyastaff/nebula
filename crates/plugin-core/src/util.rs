//! Shared utilities for core plugin actions.

use serde_json::Value;

/// Local extension on [`serde_json::Value`] to get a readable type name for
/// error messages without pulling in an extra crate.
pub(crate) trait ValueTypeNameStr {
    fn type_name_str(&self) -> &'static str;
}

impl ValueTypeNameStr for Value {
    fn type_name_str(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}
