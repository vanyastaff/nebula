//! Typed time-picker parameter.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::time::TimeOptions;
use crate::validation::ValidationRule;

/// A time-only picker parameter (no date component).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimePicker {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TimeOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl TimePicker {
    #[must_use]
    pub fn builder(key: impl Into<String>) -> TimePickerBuilder {
        TimePickerBuilder::new(key)
    }
}

#[derive(Debug)]
pub struct TimePickerBuilder {
    metadata: ParameterMetadata,
    default: Option<String>,
    options: TimeOptions,
    validation: Vec<ValidationRule>,
}

impl TimePickerBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            default: None,
            options: TimeOptions::default(),
            validation: Vec::new(),
        }
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.metadata.name = label.into();
        self
    }

    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.metadata.description = Some(desc.into());
        self
    }

    #[must_use]
    pub fn required(mut self) -> Self {
        self.metadata.required = true;
        self
    }

    #[must_use]
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.default = Some(value.into());
        self
    }

    #[must_use]
    pub fn min(mut self, value: impl Into<String>) -> Self {
        self.options.min = Some(value.into());
        self
    }

    /// Set maximum allowed time.
    #[must_use]
    pub fn max(mut self, value: impl Into<String>) -> Self {
        self.options.max = Some(value.into());
        self
    }

    /// Set display format string.
    #[must_use]
    pub fn format(mut self, value: impl Into<String>) -> Self {
        self.options.format = Some(value.into());
        self
    }

    /// Enable 24-hour format in UI.
    #[must_use]
    pub fn use_24h(mut self, value: bool) -> Self {
        self.options.use_24h = value;
        self
    }

    /// Build the time picker parameter.
    #[must_use]
    pub fn build(self) -> TimePicker {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        TimePicker {
            metadata,
            default: self.default,
            options: if self.options.min.is_some()
                || self.options.max.is_some()
                || self.options.format.is_some()
                || self.options.use_24h
            {
                Some(self.options)
            } else {
                None
            },
            display: None,
            validation: self.validation,
        }
    }
}
