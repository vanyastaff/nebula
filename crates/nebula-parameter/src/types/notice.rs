use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterKind, ParameterMetadata,
    Validatable,
};
use nebula_value::Value;

/// Parameter for displaying a notice or information to the user
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct NoticeParameter {
    /// Base parameter fields (metadata, display, validation)
    /// Note: validation is not used for notice parameters
    #[serde(flatten)]
    pub base: ParameterBase,

    /// The text content of the notice
    pub content: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<NoticeParameterOptions>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct NoticeParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Type of notice (info, warning, error, success)
    pub notice_type: Option<NoticeType>,

    /// Whether the notice can be dismissed by the user
    #[builder(default)]
    #[serde(default)]
    pub dismissible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum NoticeType {
    #[serde(rename = "info")]
    #[default]
    Info,
    #[serde(rename = "warning")]
    Warning,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "success")]
    Success,
}

impl Describable for NoticeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Notice
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

// Notice parameters implement minimal Validatable for blanket Parameter impl
impl Validatable for NoticeParameter {
    fn is_empty(&self, _value: &Value) -> bool {
        false // Notice parameters don't have values
    }
}

impl std::fmt::Display for NoticeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NoticeParameter({})", self.base.metadata.name)
    }
}

impl Displayable for NoticeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}

// Notice parameters only display information, they don't have user-editable values
// They implement only ParameterType and Displayable
