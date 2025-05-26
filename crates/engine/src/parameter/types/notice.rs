// Файл: parameter/types/notice.rs
use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::parameter::Parameter;
use crate::parameter::display::ParameterDisplay;
use crate::parameter::error::ParameterError;
use crate::parameter::metadata::ParameterMetadata;
use crate::parameter::value::ParameterValue;

/// Parameter for displaying a notice or information to the user
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
#[builder(setter(strip_option))]
pub struct NoticeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    pub value: Option<ParameterValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<NoticeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct NoticeParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<NoticeType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NoticeType {
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "success")]
    Success,
}

impl Parameter for NoticeParameter {
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
            actual_details: "Notice parameters do not accept values".to_string(),
        })
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }
}
