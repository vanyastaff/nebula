//! Text parameter type for single-line text input

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

// =============================================================================
// LengthRange
// =============================================================================

/// Length constraints for text input
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LengthRange {
    /// Minimum number of characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<usize>,

    /// Maximum number of characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<usize>,
}

impl LengthRange {
    /// Create a new length range with both min and max
    #[must_use]
    pub fn new(min: usize, max: usize) -> Self {
        Self {
            min: Some(min),
            max: Some(max),
        }
    }

    /// Create a length range with only minimum
    #[must_use]
    pub fn min_only(min: usize) -> Self {
        Self {
            min: Some(min),
            max: None,
        }
    }

    /// Create a length range with only maximum
    #[must_use]
    pub fn max_only(max: usize) -> Self {
        Self {
            min: None,
            max: Some(max),
        }
    }

    /// Create an exact length constraint (min == max)
    #[must_use]
    pub fn exact(len: usize) -> Self {
        Self {
            min: Some(len),
            max: Some(len),
        }
    }

    /// Check if a length is within this range
    #[must_use]
    pub fn contains(&self, len: usize) -> bool {
        let above_min = self.min.is_none_or(|min| len >= min);
        let below_max = self.max.is_none_or(|max| len <= max);
        above_min && below_max
    }
}

// =============================================================================
// TextCase
// =============================================================================

/// Auto-transform case for text input
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextCase {
    /// No case transformation
    #[default]
    None,
    /// Transform to UPPERCASE
    Upper,
    /// Transform to lowercase
    Lower,
}

impl TextCase {
    /// Apply case transformation to a string
    #[must_use]
    pub fn apply(&self, s: &str) -> String {
        match self {
            Self::None => s.to_string(),
            Self::Upper => s.to_uppercase(),
            Self::Lower => s.to_lowercase(),
        }
    }
}

// =============================================================================
// TextCharset
// =============================================================================

/// Allowed character set for text input
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextCharset {
    /// Any characters allowed
    #[default]
    Any,
    /// Only alphanumeric characters (a-zA-Z0-9)
    Alphanumeric,
    /// Only alphabetic characters (a-zA-Z)
    Alpha,
    /// Only numeric characters (0-9)
    Numeric,
}

impl TextCharset {
    /// Check if a character is allowed by this charset
    #[must_use]
    pub fn allows(&self, c: char) -> bool {
        match self {
            Self::Any => true,
            Self::Alphanumeric => c.is_ascii_alphanumeric(),
            Self::Alpha => c.is_ascii_alphabetic(),
            Self::Numeric => c.is_ascii_digit(),
        }
    }

    /// Check if all characters in a string are allowed
    #[must_use]
    pub fn allows_all(&self, s: &str) -> bool {
        s.chars().all(|c| self.allows(c))
    }

    /// Filter a string to only allowed characters
    #[must_use]
    pub fn filter(&self, s: &str) -> String {
        s.chars().filter(|c| self.allows(*c)).collect()
    }
}

// =============================================================================
// TextConfig
// =============================================================================

/// Configuration for text parameters
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::types::text::TextConfig;
///
/// // Simple config with length constraints
/// let config = TextConfig::new()
///     .min_length(3)
///     .max_length(20);
///
/// // Phone number with mask
/// let phone_config = TextConfig::new()
///     .mask("+7 (###) ###-##-##")
///     .charset(TextCharset::Numeric);
///
/// // Uppercase code input
/// let code_config = TextConfig::new()
///     .length(4, 8)
///     .case(TextCase::Upper)
///     .charset(TextCharset::Alphanumeric);
///
/// // URL input with prefix
/// let url_config = TextConfig::new()
///     .prefix("https://")
///     .max_length(200);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextConfig {
    /// Input mask for formatting (e.g., "+7 (###) ###-##-##")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<String>,

    /// Length constraints
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<LengthRange>,

    /// Auto-transform case
    #[serde(default, skip_serializing_if = "is_default_case")]
    pub case: TextCase,

    /// Allowed character set
    #[serde(default, skip_serializing_if = "is_default_charset")]
    pub charset: TextCharset,

    /// Fixed prefix shown in UI (e.g., "$", "https://")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,

    /// Fixed suffix shown in UI (e.g., "@gmail.com")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
}

fn is_default_case(case: &TextCase) -> bool {
    *case == TextCase::None
}

fn is_default_charset(charset: &TextCharset) -> bool {
    *charset == TextCharset::Any
}

impl TextConfig {
    /// Create a new empty config
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set input mask
    #[must_use]
    pub fn mask(mut self, mask: impl Into<String>) -> Self {
        self.mask = Some(mask.into());
        self
    }

    /// Set length range
    #[must_use]
    pub fn length(mut self, min: usize, max: usize) -> Self {
        self.length = Some(LengthRange::new(min, max));
        self
    }

    /// Set minimum length
    #[must_use]
    pub fn min_length(mut self, min: usize) -> Self {
        self.length = Some(match self.length {
            Some(mut range) => {
                range.min = Some(min);
                range
            }
            None => LengthRange::min_only(min),
        });
        self
    }

    /// Set maximum length
    #[must_use]
    pub fn max_length(mut self, max: usize) -> Self {
        self.length = Some(match self.length {
            Some(mut range) => {
                range.max = Some(max);
                range
            }
            None => LengthRange::max_only(max),
        });
        self
    }

    /// Set exact length (min == max)
    #[must_use]
    pub fn exact_length(mut self, len: usize) -> Self {
        self.length = Some(LengthRange::exact(len));
        self
    }

    /// Set case transformation
    #[must_use]
    pub fn case(mut self, case: TextCase) -> Self {
        self.case = case;
        self
    }

    /// Set to uppercase
    #[must_use]
    pub fn uppercase(mut self) -> Self {
        self.case = TextCase::Upper;
        self
    }

    /// Set to lowercase
    #[must_use]
    pub fn lowercase(mut self) -> Self {
        self.case = TextCase::Lower;
        self
    }

    /// Set allowed charset
    #[must_use]
    pub fn charset(mut self, charset: TextCharset) -> Self {
        self.charset = charset;
        self
    }

    /// Set prefix
    #[must_use]
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Set suffix
    #[must_use]
    pub fn suffix(mut self, suffix: impl Into<String>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }

    /// Get minimum length if set
    #[must_use]
    pub fn get_min_length(&self) -> Option<usize> {
        self.length.as_ref().and_then(|r| r.min)
    }

    /// Get maximum length if set
    #[must_use]
    pub fn get_max_length(&self) -> Option<usize> {
        self.length.as_ref().and_then(|r| r.max)
    }

    /// Check if length is within configured range
    #[must_use]
    pub fn is_valid_length(&self, len: usize) -> bool {
        self.length.as_ref().is_none_or(|r| r.contains(len))
    }
}

// =============================================================================
// TextParameter
// =============================================================================

/// Parameter for single-line text input
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = TextParameter::builder()
///     .key("username")
///     .name("Username")
///     .description("Enter your username")
///     .required(true)
///     .placeholder("john_doe")
///     .default("guest")
///     .config(
///         TextConfig::new()
///             .min_length(3)
///             .max_length(20)
///             .charset(TextCharset::Alphanumeric)
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Configuration for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<TextConfig>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

// =============================================================================
// TextParameter Builder
// =============================================================================

/// Builder for `TextParameter`
#[derive(Debug, Default)]
pub struct TextParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    config: Option<TextConfig>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl TextParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> TextParameterBuilder {
        TextParameterBuilder::new()
    }

    /// Get the config or default
    #[must_use]
    pub fn config_or_default(&self) -> TextConfig {
        self.config.clone().unwrap_or_default()
    }
}

impl TextParameterBuilder {
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
            config: None,
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

    /// Set the config
    #[must_use]
    pub fn config(mut self, config: TextConfig) -> Self {
        self.config = Some(config);
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

    /// Build the `TextParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<TextParameter, ParameterError> {
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

        Ok(TextParameter {
            metadata,
            default: self.default,
            config: self.config,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for TextParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Text
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for TextParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextParameter({})", self.metadata.name)
    }
}

impl Validatable for TextParameter {
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

        // Config validation
        if let Some(text) = value.as_text()
            && let Some(config) = &self.config
        {
            // Length validation
            if let Some(length) = &config.length {
                let len = text.len();
                if let Some(min) = length.min
                    && len < min
                {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Text length {len} below minimum {min}"),
                    });
                }
                if let Some(max) = length.max
                    && len > max
                {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Text length {len} above maximum {max}"),
                    });
                }
            }

            // Charset validation
            if config.charset != TextCharset::Any && !config.charset.allows_all(text.as_str()) {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!(
                        "Text contains characters not allowed by {:?} charset",
                        config.charset
                    ),
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

impl Displayable for TextParameter {
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

    // -------------------------------------------------------------------------
    // LengthRange tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_length_range_new() {
        let range = LengthRange::new(3, 20);
        assert_eq!(range.min, Some(3));
        assert_eq!(range.max, Some(20));
    }

    #[test]
    fn test_length_range_min_only() {
        let range = LengthRange::min_only(5);
        assert_eq!(range.min, Some(5));
        assert_eq!(range.max, None);
    }

    #[test]
    fn test_length_range_max_only() {
        let range = LengthRange::max_only(100);
        assert_eq!(range.min, None);
        assert_eq!(range.max, Some(100));
    }

    #[test]
    fn test_length_range_exact() {
        let range = LengthRange::exact(10);
        assert_eq!(range.min, Some(10));
        assert_eq!(range.max, Some(10));
    }

    #[test]
    fn test_length_range_contains() {
        let range = LengthRange::new(3, 10);
        assert!(!range.contains(2));
        assert!(range.contains(3));
        assert!(range.contains(5));
        assert!(range.contains(10));
        assert!(!range.contains(11));
    }

    // -------------------------------------------------------------------------
    // TextCase tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_text_case_apply() {
        assert_eq!(TextCase::None.apply("Hello"), "Hello");
        assert_eq!(TextCase::Upper.apply("Hello"), "HELLO");
        assert_eq!(TextCase::Lower.apply("Hello"), "hello");
    }

    // -------------------------------------------------------------------------
    // TextCharset tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_text_charset_allows() {
        assert!(TextCharset::Any.allows('!'));
        assert!(TextCharset::Alphanumeric.allows('a'));
        assert!(TextCharset::Alphanumeric.allows('5'));
        assert!(!TextCharset::Alphanumeric.allows('!'));
        assert!(TextCharset::Alpha.allows('z'));
        assert!(!TextCharset::Alpha.allows('5'));
        assert!(TextCharset::Numeric.allows('0'));
        assert!(!TextCharset::Numeric.allows('a'));
    }

    #[test]
    fn test_text_charset_allows_all() {
        assert!(TextCharset::Alphanumeric.allows_all("abc123"));
        assert!(!TextCharset::Alphanumeric.allows_all("abc-123"));
        assert!(TextCharset::Numeric.allows_all("12345"));
        assert!(!TextCharset::Numeric.allows_all("123a"));
    }

    #[test]
    fn test_text_charset_filter() {
        assert_eq!(TextCharset::Numeric.filter("abc123def456"), "123456");
        assert_eq!(TextCharset::Alpha.filter("abc123def"), "abcdef");
    }

    // -------------------------------------------------------------------------
    // TextConfig tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_text_config_builder_chain() {
        let config = TextConfig::new()
            .min_length(3)
            .max_length(20)
            .uppercase()
            .charset(TextCharset::Alphanumeric)
            .prefix("ID-");

        assert_eq!(config.get_min_length(), Some(3));
        assert_eq!(config.get_max_length(), Some(20));
        assert_eq!(config.case, TextCase::Upper);
        assert_eq!(config.charset, TextCharset::Alphanumeric);
        assert_eq!(config.prefix, Some("ID-".to_string()));
    }

    #[test]
    fn test_text_config_length_helper() {
        let config = TextConfig::new().length(5, 10);
        assert_eq!(config.get_min_length(), Some(5));
        assert_eq!(config.get_max_length(), Some(10));
    }

    #[test]
    fn test_text_config_exact_length() {
        let config = TextConfig::new().exact_length(6);
        assert_eq!(config.get_min_length(), Some(6));
        assert_eq!(config.get_max_length(), Some(6));
    }

    #[test]
    fn test_text_config_min_then_max() {
        let config = TextConfig::new().min_length(3).max_length(10);
        assert_eq!(config.get_min_length(), Some(3));
        assert_eq!(config.get_max_length(), Some(10));
    }

    #[test]
    fn test_text_config_is_valid_length() {
        let config = TextConfig::new().length(3, 10);
        assert!(!config.is_valid_length(2));
        assert!(config.is_valid_length(5));
        assert!(!config.is_valid_length(11));
    }

    // -------------------------------------------------------------------------
    // TextParameter tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_text_parameter_builder() {
        let param = TextParameter::builder()
            .key("username")
            .name("Username")
            .description("Enter your username")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "username");
        assert_eq!(param.metadata.name, "Username");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_text_parameter_with_config() {
        let param = TextParameter::builder()
            .key("username")
            .name("Username")
            .config(
                TextConfig::new()
                    .min_length(3)
                    .max_length(20)
                    .charset(TextCharset::Alphanumeric),
            )
            .build()
            .unwrap();

        let config = param.config.unwrap();
        assert_eq!(config.get_min_length(), Some(3));
        assert_eq!(config.get_max_length(), Some(20));
        assert_eq!(config.charset, TextCharset::Alphanumeric);
    }

    #[test]
    fn test_text_parameter_with_default() {
        let param = TextParameter::builder()
            .key("greeting")
            .name("Greeting")
            .default("Hello")
            .build()
            .unwrap();

        assert_eq!(param.default.as_ref().map(|t| t.as_str()), Some("Hello"));
    }

    #[test]
    fn test_text_parameter_missing_key() {
        let result = TextParameter::builder().name("Username").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_text_parameter_serialization() {
        let param = TextParameter::builder()
            .key("test")
            .name("Test")
            .description("A test parameter")
            .required(true)
            .build()
            .unwrap();

        let json = serde_json::to_string(&param).unwrap();
        let deserialized: TextParameter = serde_json::from_str(&json).unwrap();

        assert_eq!(param.metadata.key, deserialized.metadata.key);
        assert_eq!(param.metadata.name, deserialized.metadata.name);
    }

    #[test]
    fn test_text_parameter_validation_length() {
        let param = TextParameter::builder()
            .key("code")
            .name("Code")
            .config(TextConfig::new().length(3, 10))
            .build()
            .unwrap();

        // Too short
        let result = param.validate_sync(&Value::text("ab"));
        assert!(result.is_err());

        // Valid
        let result = param.validate_sync(&Value::text("abcde"));
        assert!(result.is_ok());

        // Too long
        let result = param.validate_sync(&Value::text("abcdefghijk"));
        assert!(result.is_err());
    }

    #[test]
    fn test_text_parameter_validation_charset() {
        let param = TextParameter::builder()
            .key("code")
            .name("Code")
            .config(TextConfig::new().charset(TextCharset::Alphanumeric))
            .build()
            .unwrap();

        // Valid
        let result = param.validate_sync(&Value::text("abc123"));
        assert!(result.is_ok());

        // Invalid - contains special char
        let result = param.validate_sync(&Value::text("abc-123"));
        assert!(result.is_err());
    }
}
