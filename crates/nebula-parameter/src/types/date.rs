//! Date parameter type for date selection

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for date selection
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = DateParameter::builder()
///     .key("birth_date")
///     .name("Birth Date")
///     .description("Enter your birth date")
///     .options(
///         DateParameterOptions::builder()
///             .format("YYYY-MM-DD")
///             .min_date("1900-01-01")
///             .max_date("2024-12-31")
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<DateParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for date parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DateParameterOptions {
    /// Date format string (e.g., "YYYY-MM-DD", "DD/MM/YYYY")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Minimum allowed date
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_date: Option<String>,

    /// Maximum allowed date
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_date: Option<String>,

    /// Show time picker alongside date
    #[serde(default)]
    pub include_time: bool,

    /// Default to today's date
    #[serde(default)]
    pub default_to_today: bool,
}

// =============================================================================
// DateParameter Builder
// =============================================================================

/// Builder for `DateParameter`
#[derive(Debug, Default)]
pub struct DateParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<String>,
    options: Option<DateParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl DateParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> DateParameterBuilder {
        DateParameterBuilder::new()
    }
}

impl DateParameterBuilder {
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
    pub fn default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: DateParameterOptions) -> Self {
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

    /// Build the `DateParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<DateParameter, ParameterError> {
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

        Ok(DateParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// DateParameterOptions Builder
// =============================================================================

/// Builder for `DateParameterOptions`
#[derive(Debug, Default)]
pub struct DateParameterOptionsBuilder {
    format: Option<String>,
    min_date: Option<String>,
    max_date: Option<String>,
    include_time: bool,
    default_to_today: bool,
}

impl DateParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> DateParameterOptionsBuilder {
        DateParameterOptionsBuilder::default()
    }
}

impl DateParameterOptionsBuilder {
    /// Set date format string
    #[must_use]
    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Set minimum allowed date
    #[must_use]
    pub fn min_date(mut self, min_date: impl Into<String>) -> Self {
        self.min_date = Some(min_date.into());
        self
    }

    /// Set maximum allowed date
    #[must_use]
    pub fn max_date(mut self, max_date: impl Into<String>) -> Self {
        self.max_date = Some(max_date.into());
        self
    }

    /// Set whether to include time picker
    #[must_use]
    pub fn include_time(mut self, include_time: bool) -> Self {
        self.include_time = include_time;
        self
    }

    /// Set whether to default to today's date
    #[must_use]
    pub fn default_to_today(mut self, default_to_today: bool) -> Self {
        self.default_to_today = default_to_today;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> DateParameterOptions {
        DateParameterOptions {
            format: self.format,
            min_date: self.min_date,
            max_date: self.max_date,
            include_time: self.include_time,
            default_to_today: self.default_to_today,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for DateParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Date
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for DateParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DateParameter({})", self.metadata.name)
    }
}

impl Validatable for DateParameter {
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

impl Displayable for DateParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl DateParameter {
    /// Validate if a string represents a valid date
    #[allow(dead_code)]
    fn is_valid_date(&self, date: &str) -> bool {
        if date.is_empty() {
            return false;
        }

        // Check for expressions (start with {{ and end with }})
        if date.starts_with("{{") && date.ends_with("}}") {
            return true;
        }

        // Basic date validation - in a real implementation you'd use a proper date library
        // This is a simplified check for ISO date format (YYYY-MM-DD)
        if date.len() == 10 && date.chars().nth(4) == Some('-') && date.chars().nth(7) == Some('-')
        {
            let parts: Vec<&str> = date.split('-').collect();
            if parts.len() == 3
                && let (Ok(year), Ok(month), Ok(day)) = (
                    parts[0].parse::<u32>(),
                    parts[1].parse::<u32>(),
                    parts[2].parse::<u32>(),
                )
            {
                return (1900..=2100).contains(&year)
                    && (1..=12).contains(&month)
                    && (1..=31).contains(&day);
            }
        }

        // Check against min/max dates if specified
        if let Some(options) = &self.options {
            if let Some(min_date) = &options.min_date
                && date < min_date.as_str()
            {
                return false;
            }
            if let Some(max_date) = &options.max_date
                && date > max_date.as_str()
            {
                return false;
            }
        }

        true
    }

    /// Get the date format for display
    #[must_use]
    pub fn get_format(&self) -> String {
        self.options
            .as_ref()
            .and_then(|opts| opts.format.as_ref())
            .cloned()
            .unwrap_or_else(|| "YYYY-MM-DD".to_string())
    }

    /// Check if this date parameter includes time
    #[must_use]
    pub fn includes_time(&self) -> bool {
        self.options.as_ref().is_some_and(|opts| opts.include_time)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_date_parameter_builder() {
        let param = DateParameter::builder()
            .key("birth_date")
            .name("Birth Date")
            .description("Enter your birth date")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "birth_date");
        assert_eq!(param.metadata.name, "Birth Date");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_date_parameter_with_options() {
        let param = DateParameter::builder()
            .key("event_date")
            .name("Event Date")
            .options(
                DateParameterOptions::builder()
                    .format("DD/MM/YYYY")
                    .min_date("2024-01-01")
                    .max_date("2024-12-31")
                    .include_time(true)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.format, Some("DD/MM/YYYY".to_string()));
        assert!(opts.include_time);
    }

    #[test]
    fn test_date_parameter_missing_key() {
        let result = DateParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
