//! Textarea parameter type for multi-line text input

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for multi-line text input
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = TextareaParameter::builder()
///     .key("description")
///     .name("Description")
///     .description("Enter a detailed description")
///     .options(
///         TextareaParameterOptions::builder()
///             .min_length(10)
///             .max_length(1000)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextareaParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TextareaParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for textarea parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextareaParameterOptions {
    /// Minimum number of characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    /// Maximum number of characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

// =============================================================================
// TextareaParameter Builder
// =============================================================================

/// Builder for `TextareaParameter`
#[derive(Debug, Default)]
pub struct TextareaParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    options: Option<TextareaParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl TextareaParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> TextareaParameterBuilder {
        TextareaParameterBuilder::new()
    }
}

impl TextareaParameterBuilder {
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
    pub fn default(mut self, default: impl Into<nebula_value::Text>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: TextareaParameterOptions) -> Self {
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

    /// Build the `TextareaParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<TextareaParameter, ParameterError> {
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

        Ok(TextareaParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// TextareaParameterOptions Builder
// =============================================================================

/// Builder for `TextareaParameterOptions`
#[derive(Debug, Default)]
pub struct TextareaParameterOptionsBuilder {
    min_length: Option<usize>,
    max_length: Option<usize>,
}

impl TextareaParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> TextareaParameterOptionsBuilder {
        TextareaParameterOptionsBuilder::default()
    }
}

impl TextareaParameterOptionsBuilder {
    /// Set minimum length
    #[must_use]
    pub fn min_length(mut self, min_length: usize) -> Self {
        self.min_length = Some(min_length);
        self
    }

    /// Set maximum length
    #[must_use]
    pub fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> TextareaParameterOptions {
        TextareaParameterOptions {
            min_length: self.min_length,
            max_length: self.max_length,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for TextareaParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Textarea
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for TextareaParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextareaParameter({})", self.metadata.name)
    }
}

impl Validatable for TextareaParameter {
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

        // Validate length constraints from options
        if let Some(text) = value.as_text()
            && let Some(opts) = &self.options
        {
            if let Some(min_len) = opts.min_length
                && text.len() < min_len
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Text too short: {} chars, minimum {}", text.len(), min_len),
                });
            }
            if let Some(max_len) = opts.max_length
                && text.len() > max_len
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Text too long: {} chars, maximum {}", text.len(), max_len),
                });
            }
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

impl Displayable for TextareaParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl TextareaParameter {
    /// Get character count for a value
    #[must_use]
    pub fn character_count(&self, value: &nebula_value::Text) -> usize {
        value.len()
    }

    /// Get remaining characters if `max_length` is set
    #[must_use]
    pub fn remaining_characters(&self, value: &nebula_value::Text) -> Option<i32> {
        if let Some(options) = &self.options
            && let Some(max_len) = options.max_length
        {
            let current = self.character_count(value);
            let max = i32::try_from(max_len).unwrap_or(i32::MAX);
            let curr = i32::try_from(current).unwrap_or(i32::MAX);
            return Some(max.saturating_sub(curr));
        }
        None
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_textarea_parameter_builder() {
        let param = TextareaParameter::builder()
            .key("description")
            .name("Description")
            .description("Enter a detailed description")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "description");
        assert_eq!(param.metadata.name, "Description");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_textarea_parameter_with_options() {
        let param = TextareaParameter::builder()
            .key("bio")
            .name("Biography")
            .options(
                TextareaParameterOptions::builder()
                    .min_length(10)
                    .max_length(500)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.min_length, Some(10));
        assert_eq!(opts.max_length, Some(500));
    }

    #[test]
    fn test_textarea_parameter_missing_key() {
        let result = TextareaParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
