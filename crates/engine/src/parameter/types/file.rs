// Файл: parameter/types/file.rs
use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::parameter::display::ParameterDisplay;
use crate::parameter::error::ParameterError;
use crate::parameter::metadata::ParameterMetadata;
use crate::parameter::validation::ParameterValidation;
use crate::parameter::value::ParameterValue;
use crate::parameter::{Parameter, validate_value};

/// Parameter for file uploads
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct FileParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_options: Option<FileParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct FileParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_formats: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_size: Option<usize>, // in bytes

    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple: Option<bool>,
}

impl Parameter for FileParameter {
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    fn get_value(&self) -> Option<&ParameterValue> {
        self.value.as_ref()
    }

    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        validate_value(self.validation(), &value)?;
        self.value = Some(value);
        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }
}
