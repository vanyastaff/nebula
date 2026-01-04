//! DateTime parameter type for date and time selection

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for date and time selection
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = DateTimeParameter::builder()
///     .key("event_datetime")
///     .name("Event Date & Time")
///     .description("Select the event date and time")
///     .options(
///         DateTimeParameterOptions::builder()
///             .format("YYYY-MM-DD HH:mm")
///             .timezone("UTC")
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateTimeParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<DateTimeParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for datetime parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DateTimeParameterOptions {
    /// `DateTime` format string (e.g., "YYYY-MM-DD HH:mm:ss", "DD/MM/YYYY HH:mm")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Minimum allowed date and time
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_datetime: Option<String>,

    /// Maximum allowed date and time
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_datetime: Option<String>,

    /// Use 12-hour format (AM/PM)
    #[serde(default)]
    pub use_12_hour: bool,

    /// Timezone handling
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,

    /// Default to current date/time
    #[serde(default)]
    pub default_to_now: bool,
}

// =============================================================================
// DateTimeParameter Builder
// =============================================================================

/// Builder for `DateTimeParameter`
#[derive(Debug, Default)]
pub struct DateTimeParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    options: Option<DateTimeParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl DateTimeParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> DateTimeParameterBuilder {
        DateTimeParameterBuilder::new()
    }
}

impl DateTimeParameterBuilder {
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
    pub fn options(mut self, options: DateTimeParameterOptions) -> Self {
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

    /// Build the `DateTimeParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<DateTimeParameter, ParameterError> {
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

        Ok(DateTimeParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// DateTimeParameterOptions Builder
// =============================================================================

/// Builder for `DateTimeParameterOptions`
#[derive(Debug, Default)]
pub struct DateTimeParameterOptionsBuilder {
    format: Option<String>,
    min_datetime: Option<String>,
    max_datetime: Option<String>,
    use_12_hour: bool,
    timezone: Option<String>,
    default_to_now: bool,
}

impl DateTimeParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> DateTimeParameterOptionsBuilder {
        DateTimeParameterOptionsBuilder::default()
    }
}

impl DateTimeParameterOptionsBuilder {
    /// Set datetime format string
    #[must_use]
    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Set minimum allowed datetime
    #[must_use]
    pub fn min_datetime(mut self, min_datetime: impl Into<String>) -> Self {
        self.min_datetime = Some(min_datetime.into());
        self
    }

    /// Set maximum allowed datetime
    #[must_use]
    pub fn max_datetime(mut self, max_datetime: impl Into<String>) -> Self {
        self.max_datetime = Some(max_datetime.into());
        self
    }

    /// Set whether to use 12-hour format
    #[must_use]
    pub fn use_12_hour(mut self, use_12_hour: bool) -> Self {
        self.use_12_hour = use_12_hour;
        self
    }

    /// Set timezone
    #[must_use]
    pub fn timezone(mut self, timezone: impl Into<String>) -> Self {
        self.timezone = Some(timezone.into());
        self
    }

    /// Set whether to default to current datetime
    #[must_use]
    pub fn default_to_now(mut self, default_to_now: bool) -> Self {
        self.default_to_now = default_to_now;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> DateTimeParameterOptions {
        DateTimeParameterOptions {
            format: self.format,
            min_datetime: self.min_datetime,
            max_datetime: self.max_datetime,
            use_12_hour: self.use_12_hour,
            timezone: self.timezone,
            default_to_now: self.default_to_now,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for DateTimeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::DateTime
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for DateTimeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DateTimeParameter({})", self.metadata.name)
    }
}

impl Validatable for DateTimeParameter {
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

impl Displayable for DateTimeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl DateTimeParameter {
    /// Get the datetime format for display
    #[must_use]
    pub fn get_format(&self) -> String {
        self.options
            .as_ref()
            .and_then(|opts| opts.format.as_ref())
            .cloned()
            .unwrap_or_else(|| "YYYY-MM-DD HH:mm:ss".to_string())
    }

    /// Check if 12-hour format should be used
    #[must_use]
    pub fn uses_12_hour_format(&self) -> bool {
        self.options.as_ref().is_some_and(|opts| opts.use_12_hour)
    }

    /// Get timezone
    #[must_use]
    pub fn get_timezone(&self) -> Option<&String> {
        self.options
            .as_ref()
            .and_then(|opts| opts.timezone.as_ref())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_datetime_parameter_builder() {
        let param = DateTimeParameter::builder()
            .key("event_datetime")
            .name("Event Date & Time")
            .description("Select the event date and time")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "event_datetime");
        assert_eq!(param.metadata.name, "Event Date & Time");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_datetime_parameter_with_options() {
        let param = DateTimeParameter::builder()
            .key("scheduled_at")
            .name("Scheduled At")
            .options(
                DateTimeParameterOptions::builder()
                    .format("YYYY-MM-DD HH:mm")
                    .timezone("UTC")
                    .use_12_hour(true)
                    .default_to_now(true)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.format, Some("YYYY-MM-DD HH:mm".to_string()));
        assert_eq!(opts.timezone, Some("UTC".to_string()));
        assert!(opts.use_12_hour);
        assert!(opts.default_to_now);
    }

    #[test]
    fn test_datetime_parameter_missing_key() {
        let result = DateTimeParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
