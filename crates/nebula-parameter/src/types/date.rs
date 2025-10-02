use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    ParameterDisplay, ParameterError, ParameterMetadata, ParameterValidation,
    ParameterValue, ParameterType, HasValue, Validatable, Displayable, ParameterKind,
};

/// Parameter for date selection
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct DateParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<DateParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct DateParameterOptions {
    /// Date format string (e.g., "YYYY-MM-DD", "DD/MM/YYYY")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Minimum allowed date
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_date: Option<String>,

    /// Maximum allowed date
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_date: Option<String>,

    /// Show time picker alongside date
    #[serde(default)]
    pub include_time: bool,

    /// Default to today's date
    #[serde(default)]
    pub default_to_today: bool,

    /// Show relative date shortcuts (Today, Yesterday, etc.)
    #[serde(default)]
    pub show_shortcuts: bool,
}

impl ParameterType for DateParameter {
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

impl HasValue for DateParameter {
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
        self.value.as_ref().map(|s| ParameterValue::Value(nebula_value::Value::text(s.clone())))
    }

    fn set_parameter_value(&mut self, value: impl Into<ParameterValue>) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Text(s)) => {
                let date_string = s.to_string();
                // Validate date format and range
                if self.is_valid_date(&date_string) {
                    self.value = Some(date_string);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Invalid date format or out of range: {}", date_string),
                    })
                }
            },
            ParameterValue::Expression(expr) => {
                // Allow expressions for dynamic dates
                self.value = Some(expr);
                Ok(())
            },
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for date parameter".to_string(),
            }),
        }
    }
}

impl Validatable for DateParameter {
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
        if date.len() == 10 && date.chars().nth(4) == Some('-') && date.chars().nth(7) == Some('-') {
            let parts: Vec<&str> = date.split('-').collect();
            if parts.len() == 3 {
                if let (Ok(year), Ok(month), Ok(day)) = (
                    parts[0].parse::<u32>(),
                    parts[1].parse::<u32>(),
                    parts[2].parse::<u32>(),
                ) {
                    return year >= 1900
                        && year <= 2100
                        && month >= 1
                        && month <= 12
                        && day >= 1
                        && day <= 31;
                }
            }
        }

        // Check against min/max dates if specified
        if let Some(options) = &self.options {
            if let Some(min_date) = &options.min_date {
                if date < min_date.as_str() {
                    return false;
                }
            }
            if let Some(max_date) = &options.max_date {
                if date > max_date.as_str() {
                    return false;
                }
            }
        }

        true
    }

    /// Get the date format for display
    pub fn get_format(&self) -> String {
        self.options
            .as_ref()
            .and_then(|opts| opts.format.as_ref())
            .cloned()
            .unwrap_or_else(|| "YYYY-MM-DD".to_string())
    }

    /// Check if this date parameter includes time
    pub fn includes_time(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.include_time)
            .unwrap_or(false)
    }
}
