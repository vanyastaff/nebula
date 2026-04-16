use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Reserved object key for explicit expression values.
pub const EXPRESSION_KEY: &str = "$expr";

/// Runtime field value representation.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    /// Plain JSON literal.
    Literal(Value),
    /// Expression string (inline `{{ ... }}` or explicit wrapper).
    Expression(String),
    /// Discriminated mode payload.
    Mode {
        /// Chosen mode key.
        mode: String,
        /// Optional mode payload.
        value: Option<Value>,
    },
}

impl FieldValue {
    /// Parse a runtime value using schema wire-format detection rules.
    pub fn from_json(value: &Value) -> Self {
        if let Some(object) = value.as_object() {
            if object.len() == 1
                && let Some(expr) = object.get(EXPRESSION_KEY).and_then(Value::as_str)
            {
                return Self::Expression(expr.to_owned());
            }

            if let Some(mode) = object.get("mode").and_then(Value::as_str) {
                return Self::Mode {
                    mode: mode.to_owned(),
                    value: object.get("value").cloned(),
                };
            }
        }

        if let Some(text) = value.as_str()
            && Self::contains_expression_marker(text)
        {
            return Self::Expression(text.to_owned());
        }

        Self::Literal(value.clone())
    }

    /// Encode into canonical JSON wire format.
    pub fn into_json(self) -> Value {
        match self {
            Self::Literal(value) => value,
            Self::Expression(expression) => json!({ EXPRESSION_KEY: expression }),
            Self::Mode { mode, value } => {
                let mut object = Map::new();
                object.insert("mode".to_owned(), Value::String(mode));
                if let Some(value) = value {
                    object.insert("value".to_owned(), value);
                }
                Value::Object(object)
            },
        }
    }

    fn contains_expression_marker(text: &str) -> bool {
        text.contains("{{") && text.contains("}}")
    }
}

/// Runtime map of field values by key.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FieldValues(HashMap<String, Value>);

impl FieldValues {
    /// Create an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set value by key.
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.0.insert(key.into(), value);
    }

    /// Borrow value by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Returns true when key exists.
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Parse value into typed runtime representation.
    pub fn get_typed(&self, key: &str) -> Option<FieldValue> {
        self.0.get(key).map(FieldValue::from_json)
    }

    /// Get string value if present and string-typed.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    /// Get boolean value if present and bool-typed.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(Value::as_bool)
    }

    /// Consume values into raw map.
    pub fn into_inner(self) -> HashMap<String, Value> {
        self.0
    }

    /// Borrow raw map by reference.
    pub fn as_map(&self) -> &HashMap<String, Value> {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{EXPRESSION_KEY, FieldValue, FieldValues};

    #[test]
    fn parses_expression_wrappers_and_inline_markers() {
        let explicit = json!({ EXPRESSION_KEY: "{{ $input.name }}" });
        let inline = json!("{{ $config.timeout }}");
        let literal = json!("plain");

        assert!(matches!(
            FieldValue::from_json(&explicit),
            FieldValue::Expression(_)
        ));
        assert!(matches!(
            FieldValue::from_json(&inline),
            FieldValue::Expression(_)
        ));
        assert!(matches!(
            FieldValue::from_json(&literal),
            FieldValue::Literal(_)
        ));
    }

    #[test]
    fn parses_mode_payload() {
        let mode = json!({
            "mode": "oauth2",
            "value": { "scope": "read" }
        });

        assert!(matches!(
            FieldValue::from_json(&mode),
            FieldValue::Mode { mode, .. } if mode == "oauth2"
        ));
    }

    #[test]
    fn field_values_exposes_typed_accessors() {
        let mut values = FieldValues::new();
        values.set("enabled", json!(true));
        values.set("text", json!("hello"));
        values.set("expr", json!("{{ $node.a }}"));

        assert_eq!(values.get_bool("enabled"), Some(true));
        assert_eq!(values.get_string("text"), Some("hello"));
        assert!(matches!(
            values.get_typed("expr"),
            Some(FieldValue::Expression(_))
        ));
    }
}
