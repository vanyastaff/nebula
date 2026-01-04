use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ParameterError;
use crate::core::{
    Describable, Displayable, Parameter, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::{Value, ValueKind};

/// Configuration for a specific mode
#[derive(Serialize)]
pub struct ModeItem {
    /// Unique key for this mode
    pub key: String,

    /// Display name for this mode
    pub name: String,

    /// Description of this mode
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Parameter for this mode
    #[serde(skip)]
    pub children: Box<dyn Parameter>,

    /// Whether this is the default mode
    #[serde(default)]
    pub default: bool,
}

impl fmt::Debug for ModeItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModeItem")
            .field("key", &self.key)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("children", &"Box<dyn Parameter>")
            .field("default", &self.default)
            .finish()
    }
}

/// Builder for ModeItem
pub struct ModeItemBuilder {
    key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    children: Option<Box<dyn Parameter>>,
    default: bool,
}

impl Default for ModeItemBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ModeItemBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: None,
            children: None,
            default: false,
        }
    }

    /// Set the mode key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the mode name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the mode description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the child parameter (required)
    #[must_use]
    pub fn child(mut self, child: Box<dyn Parameter>) -> Self {
        self.children = Some(child);
        self
    }

    /// Set whether this is the default mode
    #[must_use]
    pub fn default(mut self, is_default: bool) -> Self {
        self.default = is_default;
        self
    }

    /// Build the ModeItem
    ///
    /// # Errors
    ///
    /// Returns an error if required fields are missing
    pub fn build(self) -> Result<ModeItem, ParameterError> {
        Ok(ModeItem {
            key: self
                .key
                .ok_or_else(|| ParameterError::BuilderMissingField {
                    field: "key".into(),
                })?,
            name: self
                .name
                .ok_or_else(|| ParameterError::BuilderMissingField {
                    field: "name".into(),
                })?,
            description: self.description,
            children: self
                .children
                .ok_or_else(|| ParameterError::BuilderMissingField {
                    field: "children".into(),
                })?,
            default: self.default,
        })
    }
}

impl ModeItem {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ModeItemBuilder {
        ModeItemBuilder::new()
    }
}

/// Value for mode parameter containing the selected mode key and its value
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModeValue {
    /// Key of the selected mode
    pub key: String,
    /// Value for the selected mode
    pub value: nebula_value::Value,
}

impl From<ModeValue> for nebula_value::Value {
    fn from(mode_value: ModeValue) -> Self {
        use crate::ValueRefExt;
        let mut obj = serde_json::Map::new();
        obj.insert(
            "key".to_string(),
            nebula_value::Value::text(mode_value.key).to_json(),
        );
        obj.insert("value".to_string(), mode_value.value.to_json());

        use crate::JsonValueExt;
        serde_json::Value::Object(obj)
            .to_nebula_value()
            .unwrap_or(nebula_value::Value::Null)
    }
}

impl ModeValue {
    /// Create a new `ModeValue`
    #[must_use]
    pub fn new(key: impl Into<String>, value: nebula_value::Value) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }

    /// Create a new `ModeValue` with a string value
    #[must_use]
    pub fn text(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: nebula_value::Value::text(value.into()),
        }
    }

    /// Create a new `ModeValue` with a boolean value
    #[must_use]
    pub fn boolean(key: impl Into<String>, value: bool) -> Self {
        Self {
            key: key.into(),
            value: nebula_value::Value::boolean(value),
        }
    }

    /// Create a new `ModeValue` with an integer value
    #[must_use]
    pub fn integer(key: impl Into<String>, value: i64) -> Self {
        Self {
            key: key.into(),
            value: nebula_value::Value::integer(value),
        }
    }

    /// Create a new `ModeValue` from `ParameterValue` (`MaybeExpression<Value>`)
    #[must_use]
    pub fn from_parameter_value(
        key: impl Into<String>,
        param_value: &MaybeExpression<Value>,
    ) -> Self {
        let nebula_val = match param_value {
            MaybeExpression::Value(v) => v.clone(),
            MaybeExpression::Expression(expr) => nebula_value::Value::text(&expr.source),
        };
        Self {
            key: key.into(),
            value: nebula_val,
        }
    }
}

/// Parameter for mode selection with switching between different parameter types.
#[derive(Debug, Serialize)]
pub struct ModeParameter {
    /// Parameter metadata (flattened for cleaner JSON)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ModeValue>,

    /// Available modes with their parameters
    pub modes: Vec<ModeItem>,

    /// Display configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Builder for ModeParameter
#[derive(Default)]
pub struct ModeParameterBuilder {
    key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    default: Option<ModeValue>,
    modes: Vec<ModeItem>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl ModeParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the parameter name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the parameter description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set the placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set the hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set the default mode value
    #[must_use]
    pub fn default(mut self, default: ModeValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Add a mode
    #[must_use]
    pub fn mode(mut self, mode: ModeItem) -> Self {
        self.modes.push(mode);
        self
    }

    /// Set all modes at once
    #[must_use]
    pub fn modes(mut self, modes: Vec<ModeItem>) -> Self {
        self.modes = modes;
        self
    }

    /// Set the display configuration
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set the validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    /// Build the ModeParameter
    ///
    /// # Errors
    ///
    /// Returns an error if required fields are missing or invalid
    pub fn build(self) -> Result<ModeParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description.unwrap_or_default())
            .required(self.required)
            .maybe_placeholder(self.placeholder)
            .maybe_hint(self.hint)
            .build()?;

        Ok(ModeParameter {
            metadata,
            default: self.default,
            modes: self.modes,
            display: self.display,
            validation: self.validation,
        })
    }
}

impl ModeParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ModeParameterBuilder {
        ModeParameterBuilder::new()
    }

    /// Add a mode to this parameter
    pub fn add_mode(&mut self, mode: ModeItem) {
        self.modes.push(mode);
    }

    /// Get available modes
    #[must_use]
    pub fn available_modes(&self) -> &[ModeItem] {
        &self.modes
    }

    /// Check if a mode is available
    #[must_use]
    pub fn has_mode(&self, mode_key: &str) -> bool {
        self.modes.iter().any(|m| m.key == mode_key)
    }

    /// Get the child parameter for a specific mode key
    #[must_use]
    pub fn get_mode_child(&self, mode_key: &str) -> Option<&dyn Parameter> {
        self.modes
            .iter()
            .find(|m| m.key == mode_key)
            .map(|mode| mode.children.as_ref())
    }

    /// Get the default mode
    #[must_use]
    pub fn default_mode(&self) -> Option<&ModeItem> {
        self.modes
            .iter()
            .find(|m| m.default)
            .or_else(|| self.modes.first())
    }

    /// Get the number of modes
    #[must_use]
    pub fn mode_count(&self) -> usize {
        self.modes.len()
    }

    /// Check if there are any modes
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modes.is_empty()
    }
}

impl Describable for ModeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Mode
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl fmt::Display for ModeParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ModeParameter({})", self.metadata.name)
    }
}

impl Validatable for ModeParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Object)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        if let Some(obj) = value.as_object() {
            if let Some(inner_value) = obj.get("value") {
                match inner_value {
                    nebula_value::Value::Text(s) => s.as_str().trim().is_empty(),
                    nebula_value::Value::Null => true,
                    nebula_value::Value::Array(a) => a.is_empty(),
                    nebula_value::Value::Object(o) => o.is_empty(),
                    _ => false,
                }
            } else {
                true
            }
        } else if let Some(text) = value.as_text() {
            text.as_str().trim().is_empty()
        } else {
            value.is_null()
        }
    }
}

impl Displayable for ModeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextParameter;

    fn create_test_child(key: &str, name: &str) -> Box<dyn Parameter> {
        Box::new(
            TextParameter::builder()
                .key(key)
                .name(name)
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn test_mode_parameter_builder() {
        let text_mode = ModeItem::builder()
            .key("text")
            .name("Text Mode")
            .description("Enter text manually")
            .child(create_test_child("text_input", "Text Input"))
            .default(true)
            .build()
            .unwrap();

        let expr_mode = ModeItem::builder()
            .key("expression")
            .name("Expression Mode")
            .child(create_test_child("expr_input", "Expression"))
            .build()
            .unwrap();

        let param = ModeParameter::builder()
            .key("input_mode")
            .name("Input Mode")
            .description("Choose how to provide input")
            .mode(text_mode)
            .mode(expr_mode)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "input_mode");
        assert_eq!(param.metadata.name, "Input Mode");
        assert_eq!(param.mode_count(), 2);
        assert!(param.has_mode("text"));
        assert!(param.has_mode("expression"));
        assert!(!param.has_mode("nonexistent"));
    }

    #[test]
    fn test_mode_parameter_missing_key() {
        let result = ModeParameter::builder().name("Test").build();

        assert!(result.is_err());
    }

    #[test]
    fn test_mode_item_builder() {
        let mode = ModeItem::builder()
            .key("test")
            .name("Test Mode")
            .description("A test mode")
            .child(create_test_child("input", "Input"))
            .default(true)
            .build()
            .unwrap();

        assert_eq!(mode.key, "test");
        assert_eq!(mode.name, "Test Mode");
        assert_eq!(mode.description, Some("A test mode".to_string()));
        assert!(mode.default);
    }

    #[test]
    fn test_mode_item_missing_fields() {
        let result = ModeItem::builder()
            .key("test")
            .name("Test")
            // Missing child
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_mode_value_creation() {
        let value = ModeValue::new("text", nebula_value::Value::text("hello"));
        assert_eq!(value.key, "text");

        let text_value = ModeValue::text("text", "hello");
        assert_eq!(text_value.key, "text");

        let bool_value = ModeValue::boolean("flag", true);
        assert_eq!(bool_value.key, "flag");

        let int_value = ModeValue::integer("count", 42);
        assert_eq!(int_value.key, "count");
    }

    #[test]
    fn test_default_mode() {
        let default_mode = ModeItem::builder()
            .key("default")
            .name("Default")
            .child(create_test_child("input", "Input"))
            .default(true)
            .build()
            .unwrap();

        let other_mode = ModeItem::builder()
            .key("other")
            .name("Other")
            .child(create_test_child("input2", "Input 2"))
            .build()
            .unwrap();

        let param = ModeParameter::builder()
            .key("test")
            .name("Test")
            .mode(other_mode)
            .mode(default_mode)
            .build()
            .unwrap();

        let default = param.default_mode().unwrap();
        assert_eq!(default.key, "default");
    }

    #[test]
    fn test_get_mode_child() {
        let mode = ModeItem::builder()
            .key("text")
            .name("Text")
            .child(create_test_child("text_input", "Text Input"))
            .build()
            .unwrap();

        let param = ModeParameter::builder()
            .key("test")
            .name("Test")
            .mode(mode)
            .build()
            .unwrap();

        let child = param.get_mode_child("text");
        assert!(child.is_some());

        let nonexistent = param.get_mode_child("nonexistent");
        assert!(nonexistent.is_none());
    }

    #[test]
    fn test_empty_modes() {
        let param = ModeParameter::builder()
            .key("test")
            .name("Test")
            .build()
            .unwrap();

        assert!(param.is_empty());
        assert_eq!(param.mode_count(), 0);
        assert!(param.default_mode().is_none());
    }
}
