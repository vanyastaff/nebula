//! Checkbox parameter type for boolean input

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Boolean, Value, ValueKind};

/// Parameter for boolean checkbox
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = CheckboxParameter::builder()
///     .key("agree_terms")
///     .name("Agree to Terms")
///     .description("You must agree to the terms of service")
///     .required(true)
///     .default(false)
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckboxParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Boolean>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CheckboxParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for checkbox parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckboxParameterOptions {
    /// Custom label text for the checkbox
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Help text displayed below the checkbox
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

// =============================================================================
// CheckboxParameter Builder
// =============================================================================

/// Builder for `CheckboxParameter`
#[derive(Debug, Default)]
pub struct CheckboxParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<Boolean>,
    options: Option<CheckboxParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl CheckboxParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> CheckboxParameterBuilder {
        CheckboxParameterBuilder::new()
    }
}

impl CheckboxParameterBuilder {
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
            options: None,
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
    pub fn default(mut self, default: bool) -> Self {
        self.default = Some(Boolean::new(default));
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: CheckboxParameterOptions) -> Self {
        self.options = Some(options);
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

    /// Build the `CheckboxParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<CheckboxParameter, ParameterError> {
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

        // Apply optional metadata fields
        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(CheckboxParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// CheckboxParameterOptions Builder
// =============================================================================

/// Builder for `CheckboxParameterOptions`
#[derive(Debug, Default)]
pub struct CheckboxParameterOptionsBuilder {
    label: Option<String>,
    help_text: Option<String>,
}

impl CheckboxParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> CheckboxParameterOptionsBuilder {
        CheckboxParameterOptionsBuilder::default()
    }
}

impl CheckboxParameterOptionsBuilder {
    /// Set custom label text
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set help text
    #[must_use]
    pub fn help_text(mut self, help_text: impl Into<String>) -> Self {
        self.help_text = Some(help_text.into());
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> CheckboxParameterOptions {
        CheckboxParameterOptions {
            label: self.label,
            help_text: self.help_text,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for CheckboxParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Checkbox
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for CheckboxParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CheckboxParameter({})", self.metadata.name)
    }
}

impl Validatable for CheckboxParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Boolean)
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // Required check
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null()
    }
}

impl Displayable for CheckboxParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkbox_parameter_builder() {
        let param = CheckboxParameter::builder()
            .key("agree_terms")
            .name("Agree to Terms")
            .description("You must agree to the terms of service")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "agree_terms");
        assert_eq!(param.metadata.name, "Agree to Terms");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_checkbox_parameter_with_default() {
        let param = CheckboxParameter::builder()
            .key("enabled")
            .name("Enabled")
            .default(true)
            .build()
            .unwrap();

        assert_eq!(param.default.map(|b| b.value()), Some(true));
    }

    #[test]
    fn test_checkbox_parameter_with_options() {
        let param = CheckboxParameter::builder()
            .key("subscribe")
            .name("Subscribe")
            .options(
                CheckboxParameterOptions::builder()
                    .label("Subscribe to newsletter")
                    .help_text("We won't spam you")
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.label, Some("Subscribe to newsletter".to_string()));
        assert_eq!(opts.help_text, Some("We won't spam you".to_string()));
    }

    #[test]
    fn test_checkbox_parameter_missing_key() {
        let result = CheckboxParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_checkbox_parameter_serialization() {
        let param = CheckboxParameter::builder()
            .key("test")
            .name("Test")
            .default(false)
            .build()
            .unwrap();

        let json = serde_json::to_string(&param).unwrap();
        let deserialized: CheckboxParameter = serde_json::from_str(&json).unwrap();

        assert_eq!(param.metadata.key, deserialized.metadata.key);
        assert_eq!(param.default, deserialized.default);
    }
}
