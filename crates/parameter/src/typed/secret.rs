//! Typed secret parameter (masked sensitive text).

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::secret::SecretOptions;
use crate::validation::ValidationRule;

/// A masked text input for sensitive values like passwords and API keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Secret {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SecretOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Secret {
    #[must_use]
    pub fn builder(key: impl Into<String>) -> SecretBuilder {
        SecretBuilder::new(key)
    }
}

#[derive(Debug)]
pub struct SecretBuilder {
    metadata: ParameterMetadata,
    default: Option<String>,
    options: SecretOptions,
    validation: Vec<ValidationRule>,
}

impl SecretBuilder {
    fn new(key: impl Into<String>) -> Self {
        let mut metadata = ParameterMetadata::new(key, "");
        metadata.sensitive = true;
        Self {
            metadata,
            default: None,
            options: SecretOptions::default(),
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
    pub fn build(self) -> Secret {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Secret {
            metadata,
            default: self.default,
            options: if self.options.min_length.is_some() || self.options.max_length.is_some() {
                Some(self.options)
            } else {
                None
            },
            display: None,
            validation: self.validation,
        }
    }
}
