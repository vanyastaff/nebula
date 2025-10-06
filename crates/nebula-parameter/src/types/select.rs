use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, ParameterValue, SelectOption, Validatable,
};

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SelectParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub select_options: Option<SelectParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SelectParameterOptions {
    /// Allow multiple selections
    #[serde(default)]
    pub multiple: bool,

    /// Placeholder text when no selection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

impl Parameter for SelectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Select
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for SelectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SelectParameter({})", self.metadata.name)
    }
}

impl HasValue for SelectParameter {
    type Value = String;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }

    fn to_expression(&self) -> Option<ParameterValue> {
        self.value
            .as_ref()
            .map(|s| ParameterValue::Value(nebula_value::Value::text(s.clone())))
    }

    fn from_expression(&mut self, value: impl Into<ParameterValue>) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Text(s)) => {
                let string_value = s.to_string();
                // Validate that the value is one of the available options
                if self.is_valid_option(&string_value) {
                    self.value = Some(string_value);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Value '{}' is not a valid option", string_value),
                    })
                }
            }
            ParameterValue::Expression(expr) => {
                // Allow expressions for dynamic selection
                self.value = Some(expr);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for select parameter".to_string(),
            }),
        }
    }
}

impl Validatable for SelectParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.is_empty()
    }
}

impl Displayable for SelectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl SelectParameter {
    /// Check if a value matches one of the available options
    fn is_valid_option(&self, value: &str) -> bool {
        if value.is_empty() {
            return false;
        }

        // Check for expressions (start with {{ and end with }})
        if value.starts_with("{{") && value.ends_with("}}") {
            return true;
        }

        // Check if value matches any option's value or key
        self.options
            .iter()
            .any(|option| option.value == value || option.key == value)
    }

    /// Get option by value
    pub fn get_option_by_value(&self, value: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// Get option by key
    pub fn get_option_by_key(&self, key: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.key == key)
    }

    /// Get the display name for the current value
    pub fn get_display_name(&self) -> Option<String> {
        if let Some(value) = &self.value {
            if let Some(option) = self.get_option_by_value(value) {
                return Some(option.name.clone());
            }
        }
        None
    }
}
