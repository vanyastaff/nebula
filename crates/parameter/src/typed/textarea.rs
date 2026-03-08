//! Typed textarea parameter (multiline text input).

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::textarea::TextareaOptions;
use crate::validation::ValidationRule;

/// A multi-line text input parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Textarea {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TextareaOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Textarea {
    #[must_use]
    pub fn builder(key: impl Into<String>) -> TextareaBuilder {
        TextareaBuilder::new(key)
    }
}

#[derive(Debug)]
pub struct TextareaBuilder {
    metadata: ParameterMetadata,
    default: Option<String>,
    options: TextareaOptions,
    validation: Vec<ValidationRule>,
}

impl TextareaBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            default: None,
            options: TextareaOptions::default(),
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
    pub fn min_length(mut self, value: usize) -> Self {
        self.options.min_length = Some(value);
        self.validation.push(ValidationRule::min_length(value));
        self
    }

    #[must_use]
    pub fn max_length(mut self, value: usize) -> Self {
        self.options.max_length = Some(value);
        self.validation.push(ValidationRule::max_length(value));
        self
    }

    #[must_use]
    pub fn rows(mut self, value: u32) -> Self {
        self.options.rows = Some(value);
        self
    }

    #[must_use]
    pub fn build(self) -> Textarea {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Textarea {
            metadata,
            default: self.default,
            options: if self.options.min_length.is_some()
                || self.options.max_length.is_some()
                || self.options.rows.is_some()
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
