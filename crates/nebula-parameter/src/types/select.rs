//! Select parameter type for single-choice dropdown selection

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, SelectOption, Validatable,
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
///     .key("auth_type")
///     .name("Authentication Type")
///     .description("Choose authentication method")
///     .options(vec![
///         SelectOption::new("api_key", "API Key", "api_key"),
///         SelectOption::new("oauth", "OAuth 2.0", "oauth"),
///         SelectOption::new("basic", "Basic Auth", "basic"),
///     ])
///     .default("api_key")
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectParameter {
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
    pub select_options: Option<SelectParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for select parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelectParameterOptions {
    /// Allow multiple selections
    #[serde(default)]
    pub multiple: bool,

    /// Placeholder text when no selection
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

// =============================================================================
// SelectParameter Builder
// =============================================================================

/// Builder for `SelectParameter`
#[derive(Debug, Default)]
pub struct SelectParameterBuilder {
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
    select_options: Option<SelectParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl SelectParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> SelectParameterBuilder {
        SelectParameterBuilder::new()
    }
}

impl SelectParameterBuilder {
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
            select_options: None,
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

    /// Set select-specific options
    #[must_use]
    pub fn select_options(mut self, select_options: SelectParameterOptions) -> Self {
        self.select_options = Some(select_options);
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

    /// Build the `SelectParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<SelectParameter, ParameterError> {
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

        Ok(SelectParameter {
            metadata,
            default: self.default,
            options: self.options,
            select_options: self.select_options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// SelectParameterOptions Builder
// =============================================================================

/// Builder for `SelectParameterOptions`
#[derive(Debug, Default)]
pub struct SelectParameterOptionsBuilder {
    multiple: bool,
    placeholder: Option<String>,
}

impl SelectParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> SelectParameterOptionsBuilder {
        SelectParameterOptionsBuilder::default()
    }
}

impl SelectParameterOptionsBuilder {
    /// Set whether multiple selections are allowed
    #[must_use]
    pub fn multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> SelectParameterOptions {
        SelectParameterOptions {
            multiple: self.multiple,
            placeholder: self.placeholder,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for SelectParameter {
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

        // Validate that the value is one of the available options
        if let Some(text) = value.as_text()
            && !self.is_valid_option(text.as_str())
        {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: format!("Value '{}' is not a valid option", text.as_str()),
            });
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some_and(|s| s.is_empty())
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_parameter_builder() {
        let param = SelectParameter::builder()
            .key("auth_type")
            .name("Authentication Type")
            .description("Choose authentication method")
            .required(true)
            .options(vec![
                SelectOption::new("api_key", "API Key", "api_key"),
                SelectOption::new("oauth", "OAuth 2.0", "oauth"),
            ])
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "auth_type");
        assert_eq!(param.metadata.name, "Authentication Type");
        assert!(param.metadata.required);
        assert_eq!(param.options.len(), 2);
    }

    #[test]
    fn test_select_parameter_with_default() {
        let param = SelectParameter::builder()
            .key("priority")
            .name("Priority")
            .options(vec![
                SelectOption::new("low", "Low", "low"),
                SelectOption::new("high", "High", "high"),
            ])
            .default("low")
            .build()
            .unwrap();

        assert_eq!(param.default.as_ref().map(|t| t.as_str()), Some("low"));
    }

    #[test]
    fn test_select_parameter_add_option() {
        let param = SelectParameter::builder()
            .key("color")
            .name("Color")
            .option(SelectOption::new("red", "Red", "red"))
            .option(SelectOption::new("blue", "Blue", "blue"))
            .build()
            .unwrap();

        assert_eq!(param.options.len(), 2);
    }

    #[test]
    fn test_select_parameter_missing_key() {
        let result = SelectParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_select_parameter_get_option() {
        let param = SelectParameter::builder()
            .key("size")
            .name("Size")
            .options(vec![
                SelectOption::new("sm", "Small", "small"),
                SelectOption::new("lg", "Large", "large"),
            ])
            .build()
            .unwrap();

        let option = param.get_option_by_key("sm");
        assert!(option.is_some());
        assert_eq!(option.unwrap().name, "Small");

        let option = param.get_option_by_value("large");
        assert!(option.is_some());
        assert_eq!(option.unwrap().key, "lg");
    }

    #[test]
    fn test_select_parameter_serialization() {
        let param = SelectParameter::builder()
            .key("test")
            .name("Test")
            .options(vec![SelectOption::new("a", "A", "a")])
            .build()
            .unwrap();

        let json = serde_json::to_string(&param).unwrap();
        let deserialized: SelectParameter = serde_json::from_str(&json).unwrap();

        assert_eq!(param.metadata.key, deserialized.metadata.key);
        assert_eq!(param.options.len(), deserialized.options.len());
    }
}
