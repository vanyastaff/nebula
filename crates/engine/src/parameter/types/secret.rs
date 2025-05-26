use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::parameter::display::ParameterDisplay;
use crate::parameter::error::ParameterError;
use crate::parameter::metadata::ParameterMetadata;
use crate::parameter::validation::ParameterValidation;
use crate::parameter::value::ParameterValue;
use crate::parameter::{Parameter, validate_value};

/// Parameter for password or sensitive inputs
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct SecretParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_options: Option<SecretParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct SecretParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

impl Parameter for SecretParameter {
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
