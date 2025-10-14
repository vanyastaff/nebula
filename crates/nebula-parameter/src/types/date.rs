use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};

/// Parameter for date selection
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct DateParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current value of the parameter
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<DateParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
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
}

impl Parameter for DateParameter {
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

impl Validatable for DateParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
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
