use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    ParameterDisplay, ParameterError, ParameterMetadata, ParameterValidation,
    ParameterValue, ParameterType, HasValue, Validatable, Displayable, ParameterKind,
};

/// Parameter for time selection
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct TimeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<TimeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
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
    #[serde(default)]
    pub include_seconds: bool,

    /// Use 12-hour format (AM/PM)
    #[serde(default)]
    pub use_12_hour: bool,

    /// Time step in minutes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_minutes: Option<u32>,
}

impl ParameterType for TimeParameter {
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

impl HasValue for TimeParameter {
    type Value = String;

    fn get_value(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_value_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default_value(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear_value(&mut self) {
        self.value = None;
    }

    fn get_parameter_value(&self) -> Option<ParameterValue> {
        self.value.as_ref().map(|s| ParameterValue::Value(nebula_value::Value::String(s.clone().into())))
    }

    fn set_parameter_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        match value {
            ParameterValue::Value(nebula_value::Value::String(s)) => {
                let time_string = s.to_string();
                // Validate time format and range
                if self.is_valid_time(&time_string) {
                    self.value = Some(time_string);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Invalid time format or out of range: {}", time_string),
                    })
                }
            },
            ParameterValue::Expression(expr) => {
                // Allow expressions for dynamic times
                self.value = Some(expr);
                Ok(())
            },
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for time parameter".to_string(),
            }),
        }
    }
}

impl Validatable for TimeParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn value_to_json(&self, value: &Self::Value) -> serde_json::Value {
        serde_json::Value::String(value.clone())
    }

    fn is_empty_value(&self, value: &Self::Value) -> bool {
        value.is_empty()
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
        if let Some(options) = &self.options {
            if let Some(_format) = &options.format {
                // Custom format validation would go here
                // For now, just do basic validation
            }
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
    pub fn get_format(&self) -> String {
        self.options
            .as_ref()
            .and_then(|opts| opts.format.as_ref())
            .cloned()
            .unwrap_or_else(|| "HH:mm".to_string())
    }

    /// Check if seconds should be included
    pub fn includes_seconds(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.include_seconds)
            .unwrap_or(false)
    }

    /// Check if 12-hour format should be used
    pub fn uses_12_hour_format(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.use_12_hour)
            .unwrap_or(false)
    }

    /// Get the step in minutes
    pub fn get_step_minutes(&self) -> u32 {
        self.options
            .as_ref()
            .and_then(|opts| opts.step_minutes)
            .unwrap_or(1)
    }
}
