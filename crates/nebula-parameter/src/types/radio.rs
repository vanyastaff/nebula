use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, SelectOption, Validatable,
};

/// Parameter for selecting a single option from radio buttons
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RadioParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub radio_options: Option<RadioParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RadioParameterOptions {
    /// Show "other" option with text input
    #[serde(default)]
    pub allow_other: bool,

    /// Label for the "other" option
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_label: Option<String>,
}

impl ParameterType for RadioParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Radio
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for RadioParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RadioParameter({})", self.metadata.name)
    }
}

impl HasValue for RadioParameter {
    type Value = String;

    fn get_value(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_value_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default_value(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear_value(&mut self) {
        self.value = None;
    }

    fn get_parameter_value(&self) -> Option<ParameterValue> {
        self.value
            .as_ref()
            .map(|s| ParameterValue::Value(nebula_value::Value::text(s.clone())))
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Text(s)) => {
                let string_value = s.to_string();
                // Validate that the value is one of the available options or "other"
                if self.is_valid_option(&string_value) {
                    self.value = Some(string_value);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Value '{}' is not a valid radio option", string_value),
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
                reason: "Expected string value for radio parameter".to_string(),
            }),
        }
    }
}

impl Validatable for RadioParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty_value(&self, value: &Self::Value) -> bool {
        value.is_empty()
    }
}

impl Displayable for RadioParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl RadioParameter {
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
        let is_standard_option = self
            .options
            .iter()
            .any(|option| option.value == value || option.key == value);

        if is_standard_option {
            return true;
        }

        // Check if "other" is allowed and this might be an "other" value
        if let Some(radio_options) = &self.radio_options {
            if radio_options.allow_other {
                // For "other" option, we accept any non-empty string
                return !value.is_empty();
            }
        }

        false
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
            // If not found in options and "other" is allowed, return as-is
            if let Some(radio_options) = &self.radio_options {
                if radio_options.allow_other {
                    return Some(value.clone());
                }
            }
        }
        None
    }

    /// Check if "other" option is allowed
    pub fn allows_other(&self) -> bool {
        self.radio_options
            .as_ref()
            .map(|opts| opts.allow_other)
            .unwrap_or(false)
    }

    /// Get the "other" option label
    pub fn get_other_label(&self) -> String {
        self.radio_options
            .as_ref()
            .and_then(|opts| opts.other_label.as_ref())
            .cloned()
            .unwrap_or_else(|| "Other".to_string())
    }
}
