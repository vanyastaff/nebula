use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, ParameterValue, Validatable,
};
use nebula_value::Value;

/// Parameter for date and time selection
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct DateTimeParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<DateTimeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct DateTimeParameterOptions {
    /// `DateTime` format string (e.g., "YYYY-MM-DD HH:mm:ss", "DD/MM/YYYY HH:mm")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Minimum allowed date and time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_datetime: Option<String>,

    /// Maximum allowed date and time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_datetime: Option<String>,

    /// Use 12-hour format (AM/PM)
    #[builder(default)]
    #[serde(default)]
    pub use_12_hour: bool,

    /// Timezone handling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,

    /// Default to current date/time
    #[builder(default)]
    #[serde(default)]
    pub default_to_now: bool,
}

impl Parameter for DateTimeParameter {
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

impl ParameterValue for DateTimeParameter {
    fn validate_value(
        &self,
        value: &Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ParameterError>> + Send + '_>>
    {
        let value = value.clone();
        Box::pin(async move { self.validate(&value).await })
    }

    fn accepts_value(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().is_some()
    }

    fn expected_type(&self) -> &'static str {
        "datetime"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl DateTimeParameter {
    /// Validate if a string represents a valid datetime
    fn is_valid_datetime(&self, datetime: &str) -> bool {
        if datetime.is_empty() {
            return false;
        }

        // Check for expressions (start with {{ and end with }})
        if datetime.starts_with("{{") && datetime.ends_with("}}") {
            return true;
        }

        // Basic datetime validation - supports ISO format (YYYY-MM-DD HH:mm:ss)
        if self.validate_iso_datetime(datetime) {
            // Check against min/max datetime if specified
            if let Some(options) = &self.options {
                if let Some(min_datetime) = &options.min_datetime
                    && datetime < min_datetime.as_str()
                {
                    return false;
                }
                if let Some(max_datetime) = &options.max_datetime
                    && datetime > max_datetime.as_str()
                {
                    return false;
                }
            }
            return true;
        }

        false
    }

    /// Basic ISO datetime format validation
    fn validate_iso_datetime(&self, datetime: &str) -> bool {
        // Support formats like:
        // YYYY-MM-DD HH:mm:ss
        // YYYY-MM-DD HH:mm
        // YYYY-MM-DDTHH:mm:ss
        // YYYY-MM-DDTHH:mm:ssZ

        // Simple regex-like validation for basic ISO formats
        if datetime.len() < 16 {
            // Minimum: "YYYY-MM-DD HH:mm"
            return false;
        }

        // Split on space or 'T'
        let parts: Vec<&str> = if datetime.contains(' ') {
            datetime.split(' ').collect()
        } else if datetime.contains('T') {
            datetime.split('T').collect()
        } else {
            return false;
        };

        if parts.len() != 2 {
            return false;
        }

        // Validate date part (YYYY-MM-DD)
        let date_parts: Vec<&str> = parts[0].split('-').collect();
        if date_parts.len() != 3 {
            return false;
        }

        // Basic validation for year, month, day
        if let (Ok(year), Ok(month), Ok(day)) = (
            date_parts[0].parse::<u32>(),
            date_parts[1].parse::<u32>(),
            date_parts[2].parse::<u32>(),
        ) {
            if !(1900..=2100).contains(&year)
                || !(1..=12).contains(&month)
                || !(1..=31).contains(&day)
            {
                return false;
            }
        } else {
            return false;
        }

        // Validate time part (HH:mm or HH:mm:ss)
        let mut time_part = parts[1];

        // Remove timezone suffix if present
        if time_part.ends_with('Z') {
            time_part = &time_part[..time_part.len() - 1];
        } else if time_part.contains('+') || time_part.rfind('-').is_some_and(|pos| pos > 2) {
            // Handle timezone offsets like +03:00 or -05:00
            if let Some(tz_pos) = time_part
                .rfind('+')
                .or_else(|| time_part.rfind('-').filter(|&pos| pos > 2))
            {
                time_part = &time_part[..tz_pos];
            }
        }

        let time_parts: Vec<&str> = time_part.split(':').collect();
        if time_parts.len() < 2 || time_parts.len() > 3 {
            return false;
        }

        // Validate hours, minutes, and optionally seconds
        if let Ok(hours) = time_parts[0].parse::<u32>() {
            if hours > 23 {
                return false;
            }
        } else {
            return false;
        }

        if let Ok(minutes) = time_parts[1].parse::<u32>() {
            if minutes > 59 {
                return false;
            }
        } else {
            return false;
        }

        if time_parts.len() == 3 {
            if let Ok(seconds) = time_parts[2].parse::<u32>() {
                if seconds > 59 {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }

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
