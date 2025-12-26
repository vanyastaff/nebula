use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, SelectOption, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for selecting a single option from radio buttons
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct RadioParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    /// Available options for selection
    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub radio_options: Option<RadioParameterOptions>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct RadioParameterOptions {
    /// Show "other" option with text input
    #[builder(default)]
    #[serde(default)]
    pub allow_other: bool,

    /// Label for the "other" option
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_label: Option<String>,
}

impl Describable for RadioParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Radio
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for RadioParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RadioParameter({})", self.base.metadata.name)
    }
}

impl Validatable for RadioParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.as_text().is_none_or(|s| s.is_empty())
    }
}

impl Displayable for RadioParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
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
        if let Some(radio_options) = &self.radio_options
            && radio_options.allow_other
        {
            // For "other" option, we accept any non-empty string
            return !value.is_empty();
        }

        false
    }

    /// Get option by value
    #[must_use]
    pub fn get_option_by_value(&self, value: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// Get option by key
    #[must_use]
    pub fn get_option_by_key(&self, key: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.key == key)
    }

    /// Get the display name for a given value
    #[must_use]
    pub fn get_display_name(&self, value: &str) -> Option<String> {
        if let Some(option) = self.get_option_by_value(value) {
            return Some(option.name.clone());
        }
        // If not found in options and "other" is allowed, return as-is
        if let Some(radio_options) = &self.radio_options
            && radio_options.allow_other
        {
            return Some(value.to_string());
        }
        None
    }

    /// Check if "other" option is allowed
    #[must_use]
    pub fn allows_other(&self) -> bool {
        self.radio_options
            .as_ref()
            .is_some_and(|opts| opts.allow_other)
    }

    /// Get the "other" option label
    #[must_use]
    pub fn get_other_label(&self) -> String {
        self.radio_options
            .as_ref()
            .and_then(|opts| opts.other_label.as_ref())
            .cloned()
            .unwrap_or_else(|| "Other".to_string())
    }
}
