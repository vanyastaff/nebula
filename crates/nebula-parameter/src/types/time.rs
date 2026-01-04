//! Time parameter type for time selection

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for time selection
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = TimeParameter::builder()
///     .key("start_time")
///     .name("Start Time")
///     .description("Select a start time")
///     .options(
///         TimeParameterOptions::builder()
///             .format("HH:mm")
///             .min_time("09:00")
///             .max_time("17:00")
///             .step_minutes(15)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TimeParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for time parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimeParameterOptions {
    /// Time format string (e.g., "HH:mm", "HH:mm:ss", "hh:mm a")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Minimum allowed time
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_time: Option<String>,

    /// Maximum allowed time
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time: Option<String>,

    /// Show seconds picker
    #[serde(default)]
    pub include_seconds: bool,

    /// Use 12-hour format (AM/PM)
    #[serde(default)]
    pub use_12_hour: bool,

    /// Time step in minutes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_minutes: Option<u32>,
}

// =============================================================================
// TimeParameter Builder
// =============================================================================

/// Builder for `TimeParameter`
#[derive(Debug, Default)]
pub struct TimeParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    options: Option<TimeParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl TimeParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> TimeParameterBuilder {
        TimeParameterBuilder::new()
    }
}

impl TimeParameterBuilder {
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
    pub fn options(mut self, options: TimeParameterOptions) -> Self {
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

    /// Build the `TimeParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<TimeParameter, ParameterError> {
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

        Ok(TimeParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// TimeParameterOptions Builder
// =============================================================================

/// Builder for `TimeParameterOptions`
#[derive(Debug, Default)]
pub struct TimeParameterOptionsBuilder {
    format: Option<String>,
    min_time: Option<String>,
    max_time: Option<String>,
    include_seconds: bool,
    use_12_hour: bool,
    step_minutes: Option<u32>,
}

impl TimeParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> TimeParameterOptionsBuilder {
        TimeParameterOptionsBuilder::default()
    }
}

impl TimeParameterOptionsBuilder {
    /// Set time format string
    #[must_use]
    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// Set minimum allowed time
    #[must_use]
    pub fn min_time(mut self, min_time: impl Into<String>) -> Self {
        self.min_time = Some(min_time.into());
        self
    }

    /// Set maximum allowed time
    #[must_use]
    pub fn max_time(mut self, max_time: impl Into<String>) -> Self {
        self.max_time = Some(max_time.into());
        self
    }

    /// Set whether to include seconds picker
    #[must_use]
    pub fn include_seconds(mut self, include_seconds: bool) -> Self {
        self.include_seconds = include_seconds;
        self
    }

    /// Set whether to use 12-hour format
    #[must_use]
    pub fn use_12_hour(mut self, use_12_hour: bool) -> Self {
        self.use_12_hour = use_12_hour;
        self
    }

    /// Set time step in minutes
    #[must_use]
    pub fn step_minutes(mut self, step_minutes: u32) -> Self {
        self.step_minutes = Some(step_minutes);
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> TimeParameterOptions {
        TimeParameterOptions {
            format: self.format,
            min_time: self.min_time,
            max_time: self.max_time,
            include_seconds: self.include_seconds,
            use_12_hour: self.use_12_hour,
            step_minutes: self.step_minutes,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for TimeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Time
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for TimeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimeParameter({})", self.metadata.name)
    }
}

impl Validatable for TimeParameter {
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

impl Displayable for TimeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl TimeParameter {
    /// Get the time format for display
    #[must_use]
    pub fn get_format(&self) -> String {
        self.options
            .as_ref()
            .and_then(|opts| opts.format.as_ref())
            .cloned()
            .unwrap_or_else(|| "HH:mm".to_string())
    }

    /// Check if seconds should be included
    #[must_use]
    pub fn includes_seconds(&self) -> bool {
        self.options
            .as_ref()
            .is_some_and(|opts| opts.include_seconds)
    }

    /// Check if 12-hour format should be used
    #[must_use]
    pub fn uses_12_hour_format(&self) -> bool {
        self.options.as_ref().is_some_and(|opts| opts.use_12_hour)
    }

    /// Get the step in minutes
    #[must_use]
    pub fn get_step_minutes(&self) -> u32 {
        self.options
            .as_ref()
            .and_then(|opts| opts.step_minutes)
            .unwrap_or(1)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_parameter_builder() {
        let param = TimeParameter::builder()
            .key("start_time")
            .name("Start Time")
            .description("Select a start time")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "start_time");
        assert_eq!(param.metadata.name, "Start Time");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_time_parameter_with_options() {
        let param = TimeParameter::builder()
            .key("meeting_time")
            .name("Meeting Time")
            .options(
                TimeParameterOptions::builder()
                    .format("HH:mm")
                    .min_time("09:00")
                    .max_time("17:00")
                    .step_minutes(15)
                    .use_12_hour(true)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.format, Some("HH:mm".to_string()));
        assert_eq!(opts.step_minutes, Some(15));
        assert!(opts.use_12_hour);
    }

    #[test]
    fn test_time_parameter_missing_key() {
        let result = TimeParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
