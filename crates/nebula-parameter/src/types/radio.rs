//! Radio parameter type for single selection with radio buttons

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, SelectOption, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for selecting a single option from radio buttons
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = RadioParameter::builder()
///     .key("priority")
///     .name("Priority")
///     .description("Select priority level")
///     .options(vec![
///         SelectOption::new("low", "Low", "low"),
///         SelectOption::new("medium", "Medium", "medium"),
///         SelectOption::new("high", "High", "high"),
///     ])
///     .default("medium")
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Available options for selection
    pub options: Vec<SelectOption>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radio_options: Option<RadioParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for radio parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RadioParameterOptions {
    /// Show "other" option with text input
    #[serde(default)]
    pub allow_other: bool,

    /// Label for the "other" option
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_label: Option<String>,
}

// =============================================================================
// RadioParameter Builder
// =============================================================================

/// Builder for `RadioParameter`
#[derive(Debug, Default)]
pub struct RadioParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    options: Vec<SelectOption>,
    radio_options: Option<RadioParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl RadioParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> RadioParameterBuilder {
        RadioParameterBuilder::new()
    }
}

impl RadioParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            placeholder: None,
            hint: None,
            default: None,
            options: Vec::new(),
            radio_options: None,
            display: None,
            validation: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata methods
    // -------------------------------------------------------------------------

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the display name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: impl Into<nebula_value::Text>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Set the available options
    #[must_use]
    pub fn options(mut self, options: impl IntoIterator<Item = SelectOption>) -> Self {
        self.options = options.into_iter().collect();
        self
    }

    /// Add a single option
    #[must_use]
    pub fn option(mut self, option: SelectOption) -> Self {
        self.options.push(option);
        self
    }

    /// Set radio-specific options
    #[must_use]
    pub fn radio_options(mut self, radio_options: RadioParameterOptions) -> Self {
        self.radio_options = Some(radio_options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `RadioParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<RadioParameter, ParameterError> {
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
            .description(self.description)
            .required(self.required)
            .build()?;

        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(RadioParameter {
            metadata,
            default: self.default,
            options: self.options,
            radio_options: self.radio_options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// RadioParameterOptions Builder
// =============================================================================

/// Builder for `RadioParameterOptions`
#[derive(Debug, Default)]
pub struct RadioParameterOptionsBuilder {
    allow_other: bool,
    other_label: Option<String>,
}

impl RadioParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> RadioParameterOptionsBuilder {
        RadioParameterOptionsBuilder::default()
    }
}

impl RadioParameterOptionsBuilder {
    /// Set whether to allow "other" option
    #[must_use]
    pub fn allow_other(mut self, allow_other: bool) -> Self {
        self.allow_other = allow_other;
        self
    }

    /// Set the "other" option label
    #[must_use]
    pub fn other_label(mut self, other_label: impl Into<String>) -> Self {
        self.other_label = Some(other_label.into());
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> RadioParameterOptions {
        RadioParameterOptions {
            allow_other: self.allow_other,
            other_label: self.other_label,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for RadioParameter {
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

impl Validatable for RadioParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.as_text().is_none_or(|s| s.is_empty())
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_radio_parameter_builder() {
        let param = RadioParameter::builder()
            .key("priority")
            .name("Priority")
            .description("Select priority level")
            .required(true)
            .options(vec![
                SelectOption::new("low", "Low", "low"),
                SelectOption::new("high", "High", "high"),
            ])
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "priority");
        assert_eq!(param.metadata.name, "Priority");
        assert!(param.metadata.required);
        assert_eq!(param.options.len(), 2);
    }

    #[test]
    fn test_radio_parameter_with_default() {
        let param = RadioParameter::builder()
            .key("size")
            .name("Size")
            .options(vec![
                SelectOption::new("sm", "Small", "small"),
                SelectOption::new("lg", "Large", "large"),
            ])
            .default("small")
            .build()
            .unwrap();

        assert_eq!(param.default.as_ref().map(|t| t.as_str()), Some("small"));
    }

    #[test]
    fn test_radio_parameter_with_other() {
        let param = RadioParameter::builder()
            .key("color")
            .name("Color")
            .options(vec![SelectOption::new("red", "Red", "red")])
            .radio_options(
                RadioParameterOptions::builder()
                    .allow_other(true)
                    .other_label("Custom color")
                    .build(),
            )
            .build()
            .unwrap();

        assert!(param.allows_other());
        assert_eq!(param.get_other_label(), "Custom color");
    }

    #[test]
    fn test_radio_parameter_missing_key() {
        let result = RadioParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
