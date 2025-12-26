use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, SelectOption, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for single-choice selection from a dropdown
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = SelectParameter::builder()
///     .base(ParameterBase::new(
///         ParameterMetadata::builder()
///             .key("auth_type")
///             .name("Authentication Type")
///             .description("Choose authentication method")
///             .build()?
///     ))
///     .options(vec![
///         SelectOption::new("api_key", "API Key", "api_key"),
///         SelectOption::new("oauth", "OAuth 2.0", "oauth"),
///         SelectOption::new("basic", "Basic Auth", "basic"),
///     ])
///     .default("api_key")  // &str -> Text via Into
///     .select_options(SelectParameterOptions::builder()
///         .placeholder("Select authentication...")
///         .build())
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct SelectParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(into)]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    /// Available options for selection
    #[builder(with = FromIterator::from_iter)]
    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub select_options: Option<SelectParameterOptions>,
}

/// Configuration options for select parameters
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::SelectParameterOptions;
///
/// let options = SelectParameterOptions::builder()
///     .multiple(true)
///     .placeholder("Choose an option...")  // &str -> String via Into
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct SelectParameterOptions {
    /// Allow multiple selections
    #[serde(default)]
    #[builder(default)]
    pub multiple: bool,

    /// Placeholder text when no selection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

impl Describable for SelectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Select
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for SelectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SelectParameter({})", self.base.metadata.name)
    }
}

impl Validatable for SelectParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.base.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // Required check
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.base.metadata.key.clone(),
            });
        }

        // Validate that the value is one of the available options
        if let Some(text) = value.as_text()
            && !self.is_valid_option(text.as_str())
        {
            return Err(ParameterError::InvalidValue {
                key: self.base.metadata.key.clone(),
                reason: format!("Value '{}' is not a valid option", text.as_str()),
            });
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().map(|s| s.is_empty()).unwrap_or(false)
    }
}

impl Displayable for SelectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
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
    #[must_use]
    pub fn get_option_by_value(&self, value: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// Get option by key
    #[must_use]
    pub fn get_option_by_key(&self, key: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.key == key)
    }

    /// Get the display name for a value
    #[must_use]
    pub fn get_display_name(&self, value: &nebula_value::Text) -> Option<String> {
        self.get_option_by_value(value)
            .map(|option| option.name.clone())
    }
}
