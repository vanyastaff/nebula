use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for time selection
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct TimeParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current value of the parameter
    pub value: Option<nebula_value::Text>,

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

impl HasValue for TimeParameter {
    type Value = nebula_value::Text;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

impl Validatable for TimeParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
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

#[async_trait::async_trait]
impl Expressible for TimeParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value
            .as_ref()
            .map(|s| MaybeExpression::Value(Value::Text(s.clone())))
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        match value.into() {
            MaybeExpression::Value(Value::Text(s)) => {
                if self.is_valid_time(s.as_str()) {
                    self.value = Some(s);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Invalid time format or out of range: {}", s),
                    })
                }
            }
            MaybeExpression::Expression(expr) => {
                // Allow expressions for dynamic times
                self.value = Some(nebula_value::Text::from(expr));
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for time parameter".to_string(),
            }),
        }
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
