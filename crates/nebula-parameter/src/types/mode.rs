use serde::{Deserialize, Serialize};
use std::fmt;

use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_core::ParameterKey;
use nebula_expression::MaybeExpression;
use nebula_value::{Value, ValueKind};

/// Parameter for mode selection with switching between different parameter types
#[derive(Debug, Serialize)]
pub struct ModeParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<ModeValue>,

    /// Available modes with their parameters
    pub modes: Vec<ModeItem>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

/// Configuration for a specific mode
#[derive(Serialize)]
pub struct ModeItem {
    /// Unique key for this mode
    pub key: String,

    /// Display name for this mode
    pub name: String,

    /// Description of this mode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Parameter for this mode
    #[serde(skip)] // Skip serialization for now due to trait objects
    pub children: Box<dyn Parameter>,

    /// Whether this is the default mode
    pub default: bool,
}

impl fmt::Debug for ModeItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModeItem")
            .field("key", &self.key)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("children", &"Box<dyn ParameterType>")
            .field("default", &self.default)
            .finish()
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
    pub fn new(key: impl Into<String>, value: nebula_value::Value) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }

    /// Create a new `ModeValue` with a string value
    pub fn text(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: nebula_value::Value::text(value.into()),
        }
    }

    /// Create a new `ModeValue` with a boolean value
    pub fn boolean(key: impl Into<String>, value: bool) -> Self {
        Self {
            key: key.into(),
            value: nebula_value::Value::boolean(value),
        }
    }

    /// Create a new `ModeValue` with an integer value
    pub fn integer(key: impl Into<String>, value: i64) -> Self {
        Self {
            key: key.into(),
            value: nebula_value::Value::integer(value),
        }
    }

    /// Create a new `ModeValue` from `ParameterValue` (`MaybeExpression`<Value>)
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

impl Parameter for ModeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Mode
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for ModeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
        // Mode parameter can accept text (mode key) or object (ModeValue)
        if let Some(obj) = value.as_object() {
            // Check if the inner value is empty
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

impl ModeParameter {
    /// Create a new mode parameter
    pub fn new(
        key: &str,
        name: &str,
        description: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            metadata: ParameterMetadata {
                key: ParameterKey::new(key)?,
                name: name.to_string(),
                description: description.to_string(),
                required: false,
                placeholder: Some("Select mode...".to_string()),
                hint: Some("Choose mode and configure parameters".to_string()),
            },
            default: None,
            modes: Vec::new(),
            display: None,
            validation: None,
        })
    }

    /// Add a mode to this parameter
    pub fn add_mode(&mut self, mode: ModeItem) {
        self.modes.push(mode);
    }

    /// Get available modes
    pub fn available_modes(&self) -> &[ModeItem] {
        &self.modes
    }

    /// Check if a mode is available
    pub fn has_mode(&self, mode_key: &str) -> bool {
        self.modes.iter().any(|m| m.key == mode_key)
    }

    /// Get the child parameter for a specific mode key
    pub fn get_mode_child(&self, mode_key: &str) -> Option<&Box<dyn Parameter>> {
        self.modes
            .iter()
            .find(|m| m.key == mode_key)
            .map(|mode| &mode.children)
    }

    /// Get the default mode
    pub fn default_mode(&self) -> Option<&ModeItem> {
        self.modes
            .iter()
            .find(|m| m.default)
            .or_else(|| self.modes.first())
    }
}
