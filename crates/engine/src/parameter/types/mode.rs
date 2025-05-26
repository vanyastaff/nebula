use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use strum_macros::AsRefStr;

use crate::parameter::Parameter;
use crate::parameter::display::ParameterDisplay;
use crate::parameter::error::ParameterError;
use crate::parameter::metadata::ParameterMetadata;
use crate::parameter::types::select::SelectParameter;
use crate::parameter::types::text::TextParameter;
use crate::parameter::value::ParameterValue;

/// Parameter for mode selection (like dark/light theme)
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option), pattern = "owned")]
#[serde(rename_all = "lowercase")]
pub struct ModeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ParameterValue>,

    pub modes: ModeParameters,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}
impl Parameter for ModeParameter {
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    fn get_value(&self) -> Option<&ParameterValue> {
        self.value.as_ref()
    }

    fn set_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        // Validate the value is a ModeValue type with expected mode
        if let ParameterValue::Mode(mode_value) = &value {
            // Check that the mode is valid
            if !matches!(mode_value.mode, ModeType::List | ModeType::Text) {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
                    expected_type: "ModeType::List or ModeType::Text".to_string(),
                    actual_details: format!("Invalid mode: {:?}", mode_value.mode),
                });
            }

            // Perform validation based on the mode type
            match mode_value.mode {
                ModeType::List => {
                    // For list mode, check if the value exists in the select options
                    if let Some(validation) = &self.modes.list.validation {
                        validation.validate(&mode_value.value)?;
                    }
                }
                ModeType::Text => {
                    // For text mode, use the text parameter validation
                    if let Some(validation) = &self.modes.text.validation {
                        validation.validate(&mode_value.value)?;
                    }
                }
            }
        } else {
            return Err(ParameterError::InvalidType {
                key: self.metadata.key.clone(),
                expected_type: "ParameterValue::Mode".to_string(),
                actual_details: format!("Got {:?}", value),
            });
        }

        self.value = Some(value);
        Ok(())
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }
}

// Container for the different parameter types used by different modes
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option), pattern = "owned")]
#[serde(rename_all = "lowercase")]
pub struct ModeParameters {
    /// The text parameter used when in text mode
    pub text: TextParameter,

    /// The select parameter used when in list mode
    pub list: SelectParameter,
}

/// Possible modes for the mode parameter
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Copy, Default, AsRefStr)]
#[serde(rename_all = "lowercase")]
pub enum ModeType {
    /// List/dropdown selection mode
    List,
    /// Free text input mode
    #[default]
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub struct ModeValue {
    pub mode: ModeType,
    pub value: Value,
}

impl ModeValue {
    pub fn new(mode: ModeType, value: Value) -> Self {
        Self { mode, value }
    }

    /// Create a new ModeValue in List mode
    pub fn list(value: impl Into<Value>) -> Self {
        Self {
            mode: ModeType::List,
            value: value.into(),
        }
    }

    /// Create a new ModeValue in Text mode
    pub fn text(value: impl Into<Value>) -> Self {
        Self {
            mode: ModeType::Text,
            value: value.into(),
        }
    }
}
