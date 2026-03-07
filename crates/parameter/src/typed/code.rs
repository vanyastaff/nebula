//! Typed code-editor parameter.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::code::{CodeLanguage, CodeOptions};
use crate::validation::ValidationRule;

/// A code editor parameter with syntax highlighting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Code {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CodeOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Code {
    #[must_use]
    pub fn builder(key: impl Into<String>) -> CodeBuilder {
        CodeBuilder::new(key)
    }
}

#[derive(Debug)]
pub struct CodeBuilder {
    metadata: ParameterMetadata,
    default: Option<String>,
    options: Option<CodeOptions>,
    validation: Vec<ValidationRule>,
}

impl CodeBuilder {
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
    pub fn language(mut self, language: CodeLanguage) -> Self {
        let line_numbers = self
            .options
            .as_ref()
            .map(|o| o.line_numbers)
            .unwrap_or(false);
        self.options = Some(CodeOptions {
            language,
            line_numbers,
        });
        self
    }

    #[must_use]
    pub fn line_numbers(mut self, line_numbers: bool) -> Self {
        let language = self
            .options
            .as_ref()
            .map(|o| o.language.clone())
            .unwrap_or(CodeLanguage::Json);
        self.options = Some(CodeOptions {
            language,
            line_numbers,
        });
        self
    }

    #[must_use]
    pub fn build(self) -> Code {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Code {
            metadata,
            default: self.default,
            options: self.options,
            display: None,
            validation: self.validation,
        }
    }
}
