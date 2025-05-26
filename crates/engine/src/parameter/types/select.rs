use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::{
    Parameter, ParameterDisplay, ParameterError, ParameterMetadata, ParameterOption,
    ParameterValidation, ParameterValue,
};

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct SelectParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,

    pub options: Vec<ParameterOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

impl Parameter for SelectParameter {
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    fn get_value(&self) -> Option<&ParameterValue> {
        self.value.as_ref()
    }

    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        // TODO: Validate that the value is one of the options
        if let Some(validation) = &self.validation {
            if let ParameterValue::Value(val) = &value {
                validation.validate(val)?;
            }
        }
        self.value = Some(value);
        Ok(())
    }
}
