use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for date and time selection
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct DateTimeParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<DateTimeParameterOptions>,
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

impl Describable for DateTimeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::DateTime
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for DateTimeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DateTimeParameter({})", self.base.metadata.name)
    }
}

impl Validatable for DateTimeParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.as_text().is_none_or(|s| s.is_empty())
    }
}

impl Displayable for DateTimeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
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
