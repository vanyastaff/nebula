//! Secret parameter type for password and sensitive inputs

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for password or sensitive inputs
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = SecretParameter::builder()
///     .key("api_key")
///     .name("API Key")
///     .description("Enter your API key")
///     .required(true)
///     .options(
///         SecretParameterOptions::builder()
///             .min_length(32)
///             .always_masked(true)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SecretParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for secret parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretParameterOptions {
    /// Minimum number of characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    /// Maximum number of characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,

    /// Whether the value should be masked even in API responses (for extra security)
    #[serde(default)]
    pub always_masked: bool,
}

// =============================================================================
// SecretParameter Builder
// =============================================================================

/// Builder for `SecretParameter`
#[derive(Debug, Default)]
pub struct SecretParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    options: Option<SecretParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl SecretParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> SecretParameterBuilder {
        SecretParameterBuilder::new()
    }
}

impl SecretParameterBuilder {
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
    pub fn options(mut self, options: SecretParameterOptions) -> Self {
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

    /// Build the `SecretParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<SecretParameter, ParameterError> {
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

        Ok(SecretParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// SecretParameterOptions Builder
// =============================================================================

/// Builder for `SecretParameterOptions`
#[derive(Debug, Default)]
pub struct SecretParameterOptionsBuilder {
    min_length: Option<usize>,
    max_length: Option<usize>,
    always_masked: bool,
}

impl SecretParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> SecretParameterOptionsBuilder {
        SecretParameterOptionsBuilder::default()
    }
}

impl SecretParameterOptionsBuilder {
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

    /// Set whether to always mask the value
    #[must_use]
    pub fn always_masked(mut self, always_masked: bool) -> Self {
        self.always_masked = always_masked;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> SecretParameterOptions {
        SecretParameterOptions {
            min_length: self.min_length,
            max_length: self.max_length,
            always_masked: self.always_masked,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for SecretParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Secret
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for SecretParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretParameter({})", self.metadata.name)
    }
}

impl Validatable for SecretParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
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
        if self.is_empty(value) && self.is_required() {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        // Length validation
        if let Some(text) = value.as_text()
            && let Some(options) = &self.options
        {
            let len = text.len();

            if let Some(min_length) = options.min_length
                && len < min_length
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Secret must be at least {min_length} characters, got {len}"),
                });
            }

            if let Some(max_length) = options.max_length
                && len > max_length
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Secret must be at most {max_length} characters, got {len}"),
                });
            }
        }

        Ok(())
    }

    fn is_empty(&self, value: &Value) -> bool {
        match value {
            Value::Text(t) => t.is_empty(),
            Value::Null => true,
            _ => true,
        }
    }
}

impl Displayable for SecretParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl SecretParameter {
    /// Get the value length without exposing the actual value
    #[must_use]
    pub fn value_length(value: &Value) -> Option<usize> {
        match value {
            Value::Text(t) => Some(t.len()),
            _ => None,
        }
    }

    /// Check if the secret value is set (without exposing it)
    #[must_use]
    pub fn has_value(value: &Value) -> bool {
        match value {
            Value::Text(t) => !t.is_empty(),
            _ => false,
        }
    }

    /// Create a masked representation of the value for display
    #[must_use]
    pub fn masked_value(value: &Value) -> Option<String> {
        match value {
            Value::Text(t) => Some("*".repeat(t.len().clamp(3, 8))),
            _ => None,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_parameter_builder() {
        let param = SecretParameter::builder()
            .key("api_key")
            .name("API Key")
            .description("Enter your API key")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "api_key");
        assert_eq!(param.metadata.name, "API Key");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_secret_parameter_with_options() {
        let param = SecretParameter::builder()
            .key("password")
            .name("Password")
            .options(
                SecretParameterOptions::builder()
                    .min_length(8)
                    .max_length(64)
                    .always_masked(true)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.min_length, Some(8));
        assert_eq!(opts.max_length, Some(64));
        assert!(opts.always_masked);
    }

    #[test]
    fn test_secret_parameter_missing_key() {
        let result = SecretParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_secret_masked_value() {
        let value = Value::text("mysecretpassword");
        let masked = SecretParameter::masked_value(&value);
        assert!(masked.is_some());
        assert!(masked.unwrap().chars().all(|c| c == '*'));
    }
}
