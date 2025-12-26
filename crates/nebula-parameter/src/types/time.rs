use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for time selection
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct TimeParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<TimeParameterOptions>,
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

impl Describable for TimeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Time
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for TimeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TimeParameter({})", self.base.metadata.name)
    }
}

impl Validatable for TimeParameter {
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

impl Displayable for TimeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
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
