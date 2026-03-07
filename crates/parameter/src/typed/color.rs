//! Typed color-picker parameter.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::color::{ColorFormat, ColorOptions};
use crate::validation::ValidationRule;

/// A color picker parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Color {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ColorOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Color {
    #[must_use]
    pub fn builder(key: impl Into<String>) -> ColorBuilder {
        ColorBuilder::new(key)
    }
}

#[derive(Debug)]
pub struct ColorBuilder {
    metadata: ParameterMetadata,
    default: Option<String>,
    options: Option<ColorOptions>,
    validation: Vec<ValidationRule>,
}

impl ColorBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            default: None,
            options: None,
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
    pub fn format(mut self, format: ColorFormat) -> Self {
        self.options = Some(ColorOptions { format });
        self
    }

    #[must_use]
    pub fn build(self) -> Color {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Color {
            metadata,
            default: self.default,
            options: self.options,
            display: None,
            validation: self.validation,
        }
    }
}
