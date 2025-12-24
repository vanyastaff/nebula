use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::Value;

/// Parameter for time selection
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct TimeParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<TimeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct TimeParameterOptions {
    /// Time format string (e.g., "HH:mm", "HH:mm:ss", "hh:mm a")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Minimum allowed time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_time: Option<String>,

    /// Maximum allowed time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_time: Option<String>,

    /// Show seconds picker
    #[builder(default)]
    #[serde(default)]
    pub include_seconds: bool,

    /// Use 12-hour format (AM/PM)
    #[builder(default)]
    #[serde(default)]
    pub use_12_hour: bool,

    /// Time step in minutes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_minutes: Option<u32>,
}

impl Parameter for TimeParameter {
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
    /// Validate if a string represents a valid time
    fn is_valid_time(&self, time: &str) -> bool {
        if time.is_empty() {
            return false;
        }

        // Check for expressions (start with {{ and end with }})
        if time.starts_with("{{") && time.ends_with("}}") {
            return true;
        }

        // Basic time validation - supports HH:mm and HH:mm:ss formats
        if let Some(options) = &self.options
            && let Some(_format) = &options.format
        {
            // Custom format validation would go here
            // For now, just do basic validation
        }

        // Basic validation for common time formats
        self.validate_time_format(time)
    }

    /// Basic time format validation
    fn validate_time_format(&self, time: &str) -> bool {
        // Support HH:mm and HH:mm:ss formats
        let parts: Vec<&str> = time.split(':').collect();

        if parts.len() < 2 || parts.len() > 3 {
            return false;
        }

        // Validate hours (00-23)
        if let Ok(hours) = parts[0].parse::<u32>() {
            if hours > 23 {
                return false;
            }
        } else {
            return false;
        }

        // Validate minutes (00-59)
        if let Ok(minutes) = parts[1].parse::<u32>() {
            if minutes > 59 {
                return false;
            }
        } else {
            return false;
        }

        // Validate seconds if present (00-59)
        if parts.len() == 3 {
            if let Ok(seconds) = parts[2].parse::<u32>() {
                if seconds > 59 {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }

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
