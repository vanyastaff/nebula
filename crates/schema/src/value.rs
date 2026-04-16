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

            let has_only_mode_shape = object
                .keys()
                .all(|key| key.as_str() == "mode" || key.as_str() == "value");
            if has_only_mode_shape && let Some(mode) = object.get("mode").and_then(Value::as_str) {
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
        let bytes = text.as_bytes();
        let mut index = 0;
        while index + 1 < bytes.len() {
            if bytes[index] == b'{' && bytes[index + 1] == b'{' {
                // Escaped "{{{{" should be treated as a literal.
                if index + 3 < bytes.len() && bytes[index + 2] == b'{' && bytes[index + 3] == b'{' {
                    index += 4;
                    continue;
                }
                return true;
            }
            index += 1;
        }
        false
    }
}

/// Trait for numeric extraction helpers.
pub trait Numeric: Copy + Sized + 'static {
    /// Parse value from JSON.
    fn from_json(value: &Value) -> Option<Self>;
}

impl Numeric for f64 {
    fn from_json(value: &Value) -> Option<Self> {
        value.as_f64()
    }
}

impl Numeric for i64 {
    fn from_json(value: &Value) -> Option<Self> {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
    }
}

impl Numeric for u64 {
    fn from_json(value: &Value) -> Option<Self> {
        value.as_u64()
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

    /// Remove value by key, returning previous value if any.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.0.remove(key)
    }

    /// Borrow value by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Returns true when key exists.
    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Number of values currently set.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if no values are set.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over known keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
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

    /// Get numeric value if present and convertible to `T`.
    pub fn get_number<T: Numeric>(&self, key: &str) -> Option<T> {
        self.0.get(key).and_then(T::from_json)
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
    fn escaped_braces_are_not_expression_markers() {
        let escaped = json!("{{{{ literal }}}}");
        assert!(matches!(
            FieldValue::from_json(&escaped),
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
    fn mode_with_extra_keys_is_literal() {
        let ambiguous = json!({
            "mode": "oauth2",
            "value": { "scope": "read" },
            "extra": true
        });

        assert!(matches!(
            FieldValue::from_json(&ambiguous),
            FieldValue::Literal(_)
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
        assert_eq!(values.get_number::<i64>("enabled"), None);
    }

    #[test]
    fn field_values_exposes_mutation_and_size_helpers() {
        let mut values = FieldValues::new();
        assert!(values.is_empty());
        values.set("count", json!(3));
        assert_eq!(values.len(), 1);
        assert_eq!(values.get_number::<i64>("count"), Some(3));
        assert_eq!(values.remove("count"), Some(json!(3)));
        assert!(values.is_empty());
    }
}
