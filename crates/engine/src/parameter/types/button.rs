// Файл: parameter/types/button.rs
use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::parameter::Parameter;
use crate::parameter::display::ParameterDisplay;
use crate::parameter::error::ParameterError;
use crate::parameter::metadata::ParameterMetadata;
use crate::parameter::value::ParameterValue;

/// Parameter for button actions
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct ButtonParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub button_type: Option<ButtonType>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ButtonType {
    #[serde(rename = "primary")]
    Primary,
    #[serde(rename = "secondary")]
    Secondary,
    #[serde(rename = "danger")]
    Danger,
}

impl Parameter for ButtonParameter {
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    fn get_value(&self) -> Option<&ParameterValue> {
        None // Button parameters don't have a value
    }

    fn set_value(&mut self, _value: ParameterValue) -> Result<(), ParameterError> {
        // Button parameters typically don't have a settable value
        Err(ParameterError::InvalidType {
            key: self.metadata().key.clone(),
            expected_type: "none".to_string(),
            actual_details: "Button parameters do not accept values".to_string(),
        })
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }
}
