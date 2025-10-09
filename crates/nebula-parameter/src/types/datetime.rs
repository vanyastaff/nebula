use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for date and time selection
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct DateTimeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<DateTimeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct DateTimeParameterOptions {
    /// DateTime format string (e.g., "YYYY-MM-DD HH:mm:ss", "DD/MM/YYYY HH:mm")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Minimum allowed date and time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_datetime: Option<String>,

    /// Maximum allowed date and time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_datetime: Option<String>,

    /// Use 12-hour format (AM/PM)
    #[serde(default)]
    pub use_12_hour: bool,

    /// Timezone handling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,

    /// Default to current date/time
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

impl HasValue for DateTimeParameter {
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

#[async_trait::async_trait]
impl Expressible for DateTimeParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value
            .as_ref()
            .map(|s| MaybeExpression::Value(nebula_value::Value::Text(s.clone())))
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            MaybeExpression::Value(nebula_value::Value::Text(s)) => {
                // Validate datetime format and range
                if self.is_valid_datetime(s.as_str()) {
                    self.value = Some(s);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Invalid datetime format or out of range: {}", s),
                    })
                }
            }
            MaybeExpression::Expression(expr) => {
                // Allow expressions for dynamic datetimes
                self.value = Some(nebula_value::Text::from(expr));
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for datetime parameter".to_string(),
            }),
        }
    }
}

impl Validatable for DateTimeParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.is_empty()
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
                if let Some(min_datetime) = &options.min_datetime {
                    if datetime < min_datetime.as_str() {
                        return false;
                    }
                }
                if let Some(max_datetime) = &options.max_datetime {
                    if datetime > max_datetime.as_str() {
                        return false;
                    }
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
            if year < 1900 || year > 2100 || month < 1 || month > 12 || day < 1 || day > 31 {
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
        } else if time_part.contains('+') || time_part.rfind('-').map_or(false, |pos| pos > 2) {
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
    pub fn get_format(&self) -> String {
        self.options
            .as_ref()
            .and_then(|opts| opts.format.as_ref())
            .cloned()
            .unwrap_or_else(|| "YYYY-MM-DD HH:mm:ss".to_string())
    }

    /// Check if 12-hour format should be used
    pub fn uses_12_hour_format(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.use_12_hour)
            .unwrap_or(false)
    }

    /// Get timezone
    pub fn get_timezone(&self) -> Option<&String> {
        self.options
            .as_ref()
            .and_then(|opts| opts.timezone.as_ref())
    }
}
