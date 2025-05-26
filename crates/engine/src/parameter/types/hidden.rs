use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::parameter::Parameter;
use crate::parameter::error::ParameterError;
use crate::parameter::metadata::ParameterMetadata;
use crate::parameter::value::ParameterValue;

/// Parameter that is hidden from the user interface
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct HiddenParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,
}

impl Parameter for HiddenParameter {
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    fn get_value(&self) -> Option<&ParameterValue> {
        self.value.as_ref()
    }

    fn set_value(&mut self, _value: ParameterValue) -> Result<(), ParameterError> {
        Err(ParameterError::InvalidType {
            key: self.metadata().key.clone(),
            expected_type: "none".to_string(),
            actual_details: "Hidden parameters do not accept values".to_string(),
        })
    }
}
