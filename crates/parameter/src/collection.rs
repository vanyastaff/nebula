use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::def::ParameterDef;
use crate::error::ParameterError;
use crate::validation::ValidationRule;
use crate::values::ParameterValues;

/// An ordered collection of parameter definitions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParameterCollection {
    parameters: Vec<ParameterDef>,
}

impl ParameterCollection {
    /// Create an empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a parameter definition to the collection.
    pub fn add(&mut self, param: ParameterDef) -> &mut Self {
        self.parameters.push(param);
        self
    }

    /// Add a parameter definition (builder-style, consuming).
    #[must_use]
    pub fn with(mut self, param: ParameterDef) -> Self {
        self.parameters.push(param);
        self
    }

    /// Get a parameter by index.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&ParameterDef> {
        self.parameters.get(index)
    }

    /// Get a parameter by its key.
    #[must_use]
    pub fn get_by_key(&self, key: &str) -> Option<&ParameterDef> {
        self.parameters.iter().find(|p| p.key() == key)
    }

    /// Remove and return a parameter by key.
    pub fn remove(&mut self, key: &str) -> Option<ParameterDef> {
        let idx = self.parameters.iter().position(|p| p.key() == key)?;
        Some(self.parameters.remove(idx))
    }

    /// Check whether a parameter with the given key exists.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.parameters.iter().any(|p| p.key() == key)
    }

    /// Iterate over all parameter keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.parameters.iter().map(|p| p.key())
    }

    /// The number of parameters in the collection.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parameters.len()
    }

    /// Whether the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }

    /// Iterate over all parameter definitions.
    pub fn iter(&self) -> impl Iterator<Item = &ParameterDef> {
        self.parameters.iter()
    }

    /// Iterate mutably over all parameter definitions.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut ParameterDef> {
        self.parameters.iter_mut()
    }

    /// Validate a set of values against this collection's parameter definitions.
    ///
    /// Returns `Ok(())` if all values pass validation, or `Err(errors)` with
    /// every validation failure collected (not just the first).
    ///
    /// Checks performed per parameter:
    /// 1. Required parameters must be present and non-null.
    /// 2. Present values must match the expected JSON type.
    /// 3. Declarative `ValidationRule`s are evaluated.
    /// 4. Container types (Object, List) are validated recursively.
    ///
    /// Extra keys in `values` that have no matching definition are ignored.
    pub fn validate(&self, values: &ParameterValues) -> Result<(), Vec<ParameterError>> {
        let mut errors = Vec::new();
        for param in &self.parameters {
            validate_param(param, values.get(param.key()), param.key(), &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

// ---------------------------------------------------------------------------
// Private validation helpers
// ---------------------------------------------------------------------------

fn validate_param(
    param: &ParameterDef,
    value: Option<&Value>,
    path: &str,
    errors: &mut Vec<ParameterError>,
) {
    let expected_type = param.kind().value_type();

    // Display-only parameters (Notice, Group) carry no value.
    if expected_type == "none" {
        return;
    }

    // --- Check required / missing ---
    match value {
        None => {
            if param.is_required() {
                errors.push(ParameterError::MissingValue {
                    key: path.to_owned(),
                });
            }
            return;
        }
        Some(Value::Null) => {
            if param.is_required() {
                errors.push(ParameterError::MissingValue {
                    key: path.to_owned(),
                });
            }
            return;
        }
        _ => {}
    }

    let value = value.expect("handled None above");

    // --- Type check ---
    if !value_matches_type(value, expected_type) {
        errors.push(ParameterError::InvalidType {
            key: path.to_owned(),
            expected_type: expected_type.to_owned(),
            actual_details: json_type_name(value).to_owned(),
        });
        return; // Skip rule checks if type is wrong.
    }

    // --- Validation rules ---
    for rule in param.validation_rules() {
        evaluate_rule(rule, value, path, errors);
    }

    // --- Recursive container validation ---
    match param {
        ParameterDef::Object(obj) => {
            if let Some(map) = value.as_object() {
                for field in &obj.fields {
                    let child_path = format!("{path}.{}", field.key());
                    let child_value = map.get(field.key());
                    validate_param(field, child_value, &child_path, errors);
                }
            }
        }
        ParameterDef::List(list) => {
            if let Some(arr) = value.as_array() {
                for (i, item) in arr.iter().enumerate() {
                    let child_path = format!("{path}[{i}]");
                    validate_param(&list.item_template, Some(item), &child_path, errors);
                }
            }
        }
        _ => {}
    }
}

fn value_matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "any" => true,
        _ => false,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn evaluate_rule(
    rule: &ValidationRule,
    value: &Value,
    path: &str,
    errors: &mut Vec<ParameterError>,
) {
    match rule {
        ValidationRule::MinLength { length, message } => {
            if let Some(s) = value.as_str()
                && s.len() < *length
            {
                errors.push(ParameterError::ValidationError {
                    key: path.to_owned(),
                    reason: message
                        .clone()
                        .unwrap_or_else(|| format!("must be at least {length} characters")),
                });
            }
        }
        ValidationRule::MaxLength { length, message } => {
            if let Some(s) = value.as_str()
                && s.len() > *length
            {
                errors.push(ParameterError::ValidationError {
                    key: path.to_owned(),
                    reason: message
                        .clone()
                        .unwrap_or_else(|| format!("must be at most {length} characters")),
                });
            }
        }
        ValidationRule::Pattern { message: _, .. } => {
            // TODO: evaluate regex when a regex dependency is added
        }
        ValidationRule::Min {
            value: min,
            message,
        } => {
            if let Some(n) = value.as_f64()
                && n < *min
            {
                errors.push(ParameterError::ValidationError {
                    key: path.to_owned(),
                    reason: message
                        .clone()
                        .unwrap_or_else(|| format!("must be at least {min}")),
                });
            }
        }
        ValidationRule::Max {
            value: max,
            message,
        } => {
            if let Some(n) = value.as_f64()
                && n > *max
            {
                errors.push(ParameterError::ValidationError {
                    key: path.to_owned(),
                    reason: message
                        .clone()
                        .unwrap_or_else(|| format!("must be at most {max}")),
                });
            }
        }
        ValidationRule::OneOf { values, message } => {
            if !values.contains(value) {
                errors.push(ParameterError::ValidationError {
                    key: path.to_owned(),
                    reason: message
                        .clone()
                        .unwrap_or_else(|| "value is not one of the allowed options".to_owned()),
                });
            }
        }
        ValidationRule::Custom { .. } => {
            // TODO: expression evaluation belongs in the engine layer
        }
        ValidationRule::MinItems { count, message } => {
            if let Some(arr) = value.as_array()
                && arr.len() < *count
            {
                errors.push(ParameterError::ValidationError {
                    key: path.to_owned(),
                    reason: message
                        .clone()
                        .unwrap_or_else(|| format!("must have at least {count} items")),
                });
            }
        }
        ValidationRule::MaxItems { count, message } => {
            if let Some(arr) = value.as_array()
                && arr.len() > *count
            {
                errors.push(ParameterError::ValidationError {
                    key: path.to_owned(),
                    reason: message
                        .clone()
                        .unwrap_or_else(|| format!("must have at most {count} items")),
                });
            }
        }
    }
}

impl IntoIterator for ParameterCollection {
    type Item = ParameterDef;
    type IntoIter = std::vec::IntoIter<ParameterDef>;

    fn into_iter(self) -> Self::IntoIter {
        self.parameters.into_iter()
    }
}

impl<'a> IntoIterator for &'a ParameterCollection {
    type Item = &'a ParameterDef;
    type IntoIter = std::slice::Iter<'a, ParameterDef>;

    fn into_iter(self) -> Self::IntoIter {
        self.parameters.iter()
    }
}

impl FromIterator<ParameterDef> for ParameterCollection {
    fn from_iter<I: IntoIterator<Item = ParameterDef>>(iter: I) -> Self {
        Self {
            parameters: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // Existing collection tests
    // -----------------------------------------------------------------------

    #[test]
    fn new_is_empty() {
        let col = ParameterCollection::new();
        assert!(col.is_empty());
        assert_eq!(col.len(), 0);
    }

    #[test]
    fn add_and_get() {
        let mut col = ParameterCollection::new();
        col.add(ParameterDef::Text(TextParameter::new("host", "Hostname")));
        col.add(ParameterDef::Number(NumberParameter::new("port", "Port")));

        assert_eq!(col.len(), 2);
        assert_eq!(col.get(0).unwrap().key(), "host");
        assert_eq!(col.get(1).unwrap().key(), "port");
        assert!(col.get(2).is_none());
    }

    #[test]
    fn with_builder() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        assert_eq!(col.len(), 2);
    }

    #[test]
    fn get_by_key() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("host", "Host")))
            .with(ParameterDef::Number(NumberParameter::new("port", "Port")));

        assert_eq!(col.get_by_key("port").unwrap().key(), "port");
        assert!(col.get_by_key("missing").is_none());
    }

    #[test]
    fn remove_by_key() {
        let mut col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        let removed = col.remove("a");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().key(), "a");
        assert_eq!(col.len(), 1);
        assert!(col.remove("missing").is_none());
    }

    #[test]
    fn contains() {
        let col =
            ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("host", "Host")));

        assert!(col.contains("host"));
        assert!(!col.contains("port"));
    }

    #[test]
    fn keys_iterator() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        let keys: Vec<&str> = col.keys().collect();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn iter_and_into_iter() {
        let col = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("x", "X")));

        assert_eq!(col.iter().count(), 1);

        let keys: Vec<&str> = (&col).into_iter().map(|p| p.key()).collect();
        assert_eq!(keys, vec!["x"]);

        let owned_keys: Vec<String> = col.into_iter().map(|p| p.key().to_owned()).collect();
        assert_eq!(owned_keys, vec!["x"]);
    }

    #[test]
    fn from_iterator() {
        let defs = vec![
            ParameterDef::Text(TextParameter::new("a", "A")),
            ParameterDef::Text(TextParameter::new("b", "B")),
        ];

        let col: ParameterCollection = defs.into_iter().collect();
        assert_eq!(col.len(), 2);
    }

    #[test]
    fn iter_mut_modifies_in_place() {
        let mut col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("a", "A")))
            .with(ParameterDef::Text(TextParameter::new("b", "B")));

        for param in col.iter_mut() {
            param.metadata_mut().required = true;
        }

        assert!(col.get(0).unwrap().is_required());
        assert!(col.get(1).unwrap().is_required());
    }

    #[test]
    fn partial_eq_collections() {
        let a = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("x", "X")));
        let b = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("x", "X")));
        assert_eq!(a, b);

        let c = ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("y", "Y")));
        assert_ne!(a, c);
    }

    #[test]
    fn serde_round_trip() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("host", "Host")))
            .with(ParameterDef::Number(NumberParameter::new("port", "Port")));

        let json = serde_json::to_string(&col).unwrap();
        let deserialized: ParameterCollection = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized.get(0).unwrap().key(), "host");
        assert_eq!(deserialized.get(1).unwrap().key(), "port");
    }

    // -----------------------------------------------------------------------
    // Validation tests
    // -----------------------------------------------------------------------

    fn required_text(key: &str, name: &str) -> ParameterDef {
        let mut p = TextParameter::new(key, name);
        p.metadata.required = true;
        ParameterDef::Text(p)
    }

    fn required_number(key: &str, name: &str) -> ParameterDef {
        let mut p = NumberParameter::new(key, name);
        p.metadata.required = true;
        ParameterDef::Number(p)
    }

    fn required_checkbox(key: &str, name: &str) -> ParameterDef {
        let mut p = CheckboxParameter::new(key, name);
        p.metadata.required = true;
        ParameterDef::Checkbox(p)
    }

    #[test]
    fn validate_empty_collection_always_ok() {
        let col = ParameterCollection::new();
        let values = ParameterValues::new();
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_optional_field_absent_is_ok() {
        let col =
            ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("host", "Host")));
        let values = ParameterValues::new();
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_required_field_missing() {
        let col = ParameterCollection::new().with(required_text("host", "Host"));
        let values = ParameterValues::new();
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], ParameterError::MissingValue { key } if key == "host"));
    }

    #[test]
    fn validate_required_field_null() {
        let col = ParameterCollection::new().with(required_text("host", "Host"));
        let mut values = ParameterValues::new();
        values.set("host", Value::Null);
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], ParameterError::MissingValue { key } if key == "host"));
    }

    #[test]
    fn validate_required_field_present() {
        let col = ParameterCollection::new().with(required_text("host", "Host"));
        let mut values = ParameterValues::new();
        values.set("host", json!("localhost"));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_type_mismatch_string_got_number() {
        let col =
            ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("host", "Host")));
        let mut values = ParameterValues::new();
        values.set("host", json!(42));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { key, expected_type, actual_details }
            if key == "host" && expected_type == "string" && actual_details == "number"
        ));
    }

    #[test]
    fn validate_type_mismatch_number_got_string() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Number(NumberParameter::new("port", "Port")));
        let mut values = ParameterValues::new();
        values.set("port", json!("abc"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { key, expected_type, .. }
            if key == "port" && expected_type == "number"
        ));
    }

    #[test]
    fn validate_type_mismatch_boolean_got_string() {
        let col = ParameterCollection::new().with(ParameterDef::Checkbox(CheckboxParameter::new(
            "enabled", "Enabled",
        )));
        let mut values = ParameterValues::new();
        values.set("enabled", json!("true"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { expected_type, .. } if expected_type == "boolean"
        ));
    }

    #[test]
    fn validate_correct_types_pass() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("host", "Host")))
            .with(ParameterDef::Number(NumberParameter::new("port", "Port")))
            .with(ParameterDef::Checkbox(CheckboxParameter::new("tls", "TLS")));

        let mut values = ParameterValues::new();
        values.set("host", json!("localhost"));
        values.set("port", json!(5432));
        values.set("tls", json!(true));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_select_accepts_any_type() {
        let col = ParameterCollection::new().with(ParameterDef::Select(SelectParameter::new(
            "region", "Region",
        )));

        let mut values = ParameterValues::new();
        values.set("region", json!("us-east-1"));
        assert!(col.validate(&values).is_ok());

        values.set("region", json!(42));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_min_length_violation() {
        let mut text = TextParameter::new("name", "Name");
        text.validation.push(ValidationRule::min_length(3));
        let col = ParameterCollection::new().with(ParameterDef::Text(text));

        let mut values = ParameterValues::new();
        values.set("name", json!("ab"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(
            matches!(&errs[0], ParameterError::ValidationError { key, reason }
            if key == "name" && reason.contains("at least 3"))
        );
    }

    #[test]
    fn validate_min_length_boundary() {
        let mut text = TextParameter::new("name", "Name");
        text.validation.push(ValidationRule::min_length(3));
        let col = ParameterCollection::new().with(ParameterDef::Text(text));

        let mut values = ParameterValues::new();
        values.set("name", json!("abc"));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_max_length_violation() {
        let mut text = TextParameter::new("code", "Code");
        text.validation.push(ValidationRule::max_length(5));
        let col = ParameterCollection::new().with(ParameterDef::Text(text));

        let mut values = ParameterValues::new();
        values.set("code", json!("abcdef"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(
            matches!(&errs[0], ParameterError::ValidationError { key, reason }
            if key == "code" && reason.contains("at most 5"))
        );
    }

    #[test]
    fn validate_max_length_boundary() {
        let mut text = TextParameter::new("code", "Code");
        text.validation.push(ValidationRule::max_length(5));
        let col = ParameterCollection::new().with(ParameterDef::Text(text));

        let mut values = ParameterValues::new();
        values.set("code", json!("abcde"));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_min_violation() {
        let mut num = NumberParameter::new("port", "Port");
        num.validation.push(ValidationRule::min(1.0));
        let col = ParameterCollection::new().with(ParameterDef::Number(num));

        let mut values = ParameterValues::new();
        values.set("port", json!(0));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], ParameterError::ValidationError { key, .. } if key == "port"));
    }

    #[test]
    fn validate_min_boundary() {
        let mut num = NumberParameter::new("port", "Port");
        num.validation.push(ValidationRule::min(1.0));
        let col = ParameterCollection::new().with(ParameterDef::Number(num));

        let mut values = ParameterValues::new();
        values.set("port", json!(1));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_max_violation() {
        let mut num = NumberParameter::new("port", "Port");
        num.validation.push(ValidationRule::max(65535.0));
        let col = ParameterCollection::new().with(ParameterDef::Number(num));

        let mut values = ParameterValues::new();
        values.set("port", json!(70000));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], ParameterError::ValidationError { key, .. } if key == "port"));
    }

    #[test]
    fn validate_max_boundary() {
        let mut num = NumberParameter::new("port", "Port");
        num.validation.push(ValidationRule::max(65535.0));
        let col = ParameterCollection::new().with(ParameterDef::Number(num));

        let mut values = ParameterValues::new();
        values.set("port", json!(65535));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_one_of_violation() {
        let col = ParameterCollection::new().with({
            let mut text = TextParameter::new("size", "Size");
            text.validation.push(ValidationRule::OneOf {
                values: vec![json!("S"), json!("M"), json!("L")],
                message: None,
            });
            ParameterDef::Text(text)
        });

        let mut values = ParameterValues::new();
        values.set("size", json!("XL"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(
            matches!(&errs[0], ParameterError::ValidationError { key, reason }
            if key == "size" && reason.contains("not one of"))
        );
    }

    #[test]
    fn validate_one_of_pass() {
        let col = ParameterCollection::new().with({
            let mut text = TextParameter::new("size", "Size");
            text.validation.push(ValidationRule::OneOf {
                values: vec![json!("S"), json!("M"), json!("L")],
                message: None,
            });
            ParameterDef::Text(text)
        });

        let mut values = ParameterValues::new();
        values.set("size", json!("M"));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_min_items_violation() {
        let mut list = ListParameter::new(
            "tags",
            "Tags",
            ParameterDef::Text(TextParameter::new("tag", "Tag")),
        );
        list.validation.push(ValidationRule::min_items(2));
        let col = ParameterCollection::new().with(ParameterDef::List(list));

        let mut values = ParameterValues::new();
        values.set("tags", json!(["one"]));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(
            matches!(&errs[0], ParameterError::ValidationError { key, reason }
            if key == "tags" && reason.contains("at least 2"))
        );
    }

    #[test]
    fn validate_max_items_violation() {
        let mut list = ListParameter::new(
            "tags",
            "Tags",
            ParameterDef::Text(TextParameter::new("tag", "Tag")),
        );
        list.validation.push(ValidationRule::max_items(2));
        let col = ParameterCollection::new().with(ParameterDef::List(list));

        let mut values = ParameterValues::new();
        values.set("tags", json!(["a", "b", "c"]));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(
            matches!(&errs[0], ParameterError::ValidationError { key, reason }
            if key == "tags" && reason.contains("at most 2"))
        );
    }

    #[test]
    fn validate_custom_rule_skipped() {
        let mut text = TextParameter::new("expr", "Expression");
        text.validation.push(ValidationRule::Custom {
            expression: "{{ $value > 0 }}".into(),
            message: None,
        });
        let col = ParameterCollection::new().with(ParameterDef::Text(text));

        let mut values = ParameterValues::new();
        values.set("expr", json!("anything"));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_pattern_rule_skipped() {
        let mut text = TextParameter::new("email", "Email");
        text.validation
            .push(ValidationRule::pattern(r"^.+@.+\..+$"));
        let col = ParameterCollection::new().with(ParameterDef::Text(text));

        let mut values = ParameterValues::new();
        values.set("email", json!("not-an-email"));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_collects_multiple_errors() {
        let col = ParameterCollection::new()
            .with(required_text("host", "Host"))
            .with(required_number("port", "Port"))
            .with(required_checkbox("tls", "TLS"));

        let values = ParameterValues::new();
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 3);
    }

    #[test]
    fn validate_mixed_error_types() {
        let mut text = TextParameter::new("name", "Name");
        text.metadata.required = true;
        text.validation.push(ValidationRule::min_length(3));

        let col = ParameterCollection::new()
            .with(ParameterDef::Text(text))
            .with(required_number("port", "Port"));

        let mut values = ParameterValues::new();
        values.set("name", json!("ab")); // min_length violation
        // "port" missing -> MissingValue

        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 2);
        assert!(
            errs.iter()
                .any(|e| matches!(e, ParameterError::ValidationError { key, .. } if key == "name"))
        );
        assert!(
            errs.iter()
                .any(|e| matches!(e, ParameterError::MissingValue { key } if key == "port"))
        );
    }

    #[test]
    fn validate_notice_is_skipped() {
        let col = ParameterCollection::new().with(ParameterDef::Notice(NoticeParameter::new(
            "info",
            "Info",
            NoticeType::Info,
            "Hello",
        )));

        let values = ParameterValues::new();
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_nested_object_valid() {
        let mut host = TextParameter::new("host", "Host");
        host.metadata.required = true;
        let mut port = NumberParameter::new("port", "Port");
        port.metadata.required = true;

        let obj = ObjectParameter::new("connection", "Connection")
            .with_field(ParameterDef::Text(host))
            .with_field(ParameterDef::Number(port));

        let col = ParameterCollection::new().with(ParameterDef::Object(obj));

        let mut values = ParameterValues::new();
        values.set("connection", json!({"host": "localhost", "port": 5432}));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_nested_object_missing_field() {
        let mut host = TextParameter::new("host", "Host");
        host.metadata.required = true;

        let obj =
            ObjectParameter::new("connection", "Connection").with_field(ParameterDef::Text(host));

        let col = ParameterCollection::new().with(ParameterDef::Object(obj));

        let mut values = ParameterValues::new();
        values.set("connection", json!({}));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::MissingValue { key } if key == "connection.host"
        ));
    }

    #[test]
    fn validate_nested_object_type_mismatch() {
        let obj = ObjectParameter::new("conn", "Conn")
            .with_field(ParameterDef::Number(NumberParameter::new("port", "Port")));

        let col = ParameterCollection::new().with(ParameterDef::Object(obj));

        let mut values = ParameterValues::new();
        values.set("conn", json!({"port": "abc"}));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { key, expected_type, .. }
            if key == "conn.port" && expected_type == "number"
        ));
    }

    #[test]
    fn validate_nested_object_wrong_top_level_type() {
        let obj = ObjectParameter::new("conn", "Conn")
            .with_field(ParameterDef::Text(TextParameter::new("host", "Host")));

        let col = ParameterCollection::new().with(ParameterDef::Object(obj));

        let mut values = ParameterValues::new();
        values.set("conn", json!("not an object"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { key, expected_type, .. }
            if key == "conn" && expected_type == "object"
        ));
    }

    #[test]
    fn validate_list_items_valid() {
        let list = ListParameter::new(
            "emails",
            "Emails",
            ParameterDef::Text(TextParameter::new("email", "Email")),
        );

        let col = ParameterCollection::new().with(ParameterDef::List(list));

        let mut values = ParameterValues::new();
        values.set("emails", json!(["a@b.com", "c@d.com"]));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_list_items_type_mismatch() {
        let list = ListParameter::new(
            "numbers",
            "Numbers",
            ParameterDef::Number(NumberParameter::new("n", "N")),
        );

        let col = ParameterCollection::new().with(ParameterDef::List(list));

        let mut values = ParameterValues::new();
        values.set("numbers", json!([1, "two", 3]));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { key, expected_type, .. }
            if key == "numbers[1]" && expected_type == "number"
        ));
    }

    #[test]
    fn validate_list_wrong_top_level_type() {
        let list = ListParameter::new(
            "items",
            "Items",
            ParameterDef::Text(TextParameter::new("item", "Item")),
        );

        let col = ParameterCollection::new().with(ParameterDef::List(list));

        let mut values = ParameterValues::new();
        values.set("items", json!("not an array"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { key, expected_type, .. }
            if key == "items" && expected_type == "array"
        ));
    }

    #[test]
    fn validate_list_with_object_items() {
        let mut host = TextParameter::new("host", "Host");
        host.metadata.required = true;

        let obj = ObjectParameter::new("entry", "Entry").with_field(ParameterDef::Text(host));

        let list = ListParameter::new("servers", "Servers", ParameterDef::Object(obj));

        let col = ParameterCollection::new().with(ParameterDef::List(list));

        let mut values = ParameterValues::new();
        values.set(
            "servers",
            json!([
                {"host": "a.com"},
                {}
            ]),
        );
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::MissingValue { key } if key == "servers[1].host"
        ));
    }

    #[test]
    fn validate_deeply_nested_object_in_list() {
        let mut inner_field = TextParameter::new("value", "Value");
        inner_field.metadata.required = true;

        let inner_obj =
            ObjectParameter::new("header", "Header").with_field(ParameterDef::Text(inner_field));

        let list = ListParameter::new("headers", "Headers", ParameterDef::Object(inner_obj));

        let outer_obj =
            ObjectParameter::new("request", "Request").with_field(ParameterDef::List(list));

        let col = ParameterCollection::new().with(ParameterDef::Object(outer_obj));

        let mut values = ParameterValues::new();
        values.set("request", json!({"headers": [{"value": "ok"}, {}]}));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::MissingValue { key } if key == "request.headers[1].value"
        ));
    }

    #[test]
    fn validate_extra_values_ignored() {
        let col =
            ParameterCollection::new().with(ParameterDef::Text(TextParameter::new("host", "Host")));

        let mut values = ParameterValues::new();
        values.set("host", json!("localhost"));
        values.set("extra", json!("ignored"));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_validation_rules_on_nested_fields() {
        let mut port = NumberParameter::new("port", "Port");
        port.validation.push(ValidationRule::min(1.0));
        port.validation.push(ValidationRule::max(65535.0));

        let obj = ObjectParameter::new("conn", "Connection").with_field(ParameterDef::Number(port));

        let col = ParameterCollection::new().with(ParameterDef::Object(obj));

        let mut values = ParameterValues::new();
        values.set("conn", json!({"port": 0}));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::ValidationError { key, .. } if key == "conn.port"
        ));
    }

    #[test]
    fn validate_empty_list_no_min_items_ok() {
        let list = ListParameter::new(
            "tags",
            "Tags",
            ParameterDef::Text(TextParameter::new("tag", "Tag")),
        );
        let col = ParameterCollection::new().with(ParameterDef::List(list));

        let mut values = ParameterValues::new();
        values.set("tags", json!([]));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_custom_error_message() {
        let mut num = NumberParameter::new("age", "Age");
        num.validation
            .push(ValidationRule::min(18.0).with_message("must be 18 or older"));
        let col = ParameterCollection::new().with(ParameterDef::Number(num));

        let mut values = ParameterValues::new();
        values.set("age", json!(16));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::ValidationError { reason, .. } if reason == "must be 18 or older"
        ));
    }

    #[test]
    fn validate_multiple_rules_on_same_param() {
        let mut num = NumberParameter::new("port", "Port");
        num.validation.push(ValidationRule::min(1.0));
        num.validation.push(ValidationRule::max(65535.0));
        let col = ParameterCollection::new().with(ParameterDef::Number(num));

        let mut values = ParameterValues::new();
        values.set("port", json!(100));
        assert!(col.validate(&values).is_ok());

        values.set("port", json!(0));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1); // only min fails

        values.set("port", json!(70000));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1); // only max fails
    }

    #[test]
    fn validate_multi_select_expects_array() {
        let col = ParameterCollection::new().with(ParameterDef::MultiSelect(
            MultiSelectParameter::new("tags", "Tags"),
        ));

        let mut values = ParameterValues::new();
        values.set("tags", json!("not-array"));
        let errs = col.validate(&values).unwrap_err();
        assert_eq!(errs.len(), 1);
        assert!(matches!(
            &errs[0],
            ParameterError::InvalidType { expected_type, .. } if expected_type == "array"
        ));

        values.set("tags", json!(["a", "b"]));
        assert!(col.validate(&values).is_ok());
    }

    #[test]
    fn validate_hidden_accepts_any() {
        let col = ParameterCollection::new()
            .with(ParameterDef::Hidden(HiddenParameter::new("token", "Token")));

        let mut values = ParameterValues::new();
        values.set("token", json!("string"));
        assert!(col.validate(&values).is_ok());

        values.set("token", json!(42));
        assert!(col.validate(&values).is_ok());

        values.set("token", json!(true));
        assert!(col.validate(&values).is_ok());
    }
}
