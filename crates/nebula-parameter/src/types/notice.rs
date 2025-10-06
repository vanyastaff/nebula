use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{Displayable, Parameter, ParameterDisplay, ParameterKind, ParameterMetadata};

/// Parameter for displaying a notice or information to the user
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct NoticeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The text content of the notice
    pub content: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<NoticeParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct NoticeParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice_type: Option<NoticeType>,

    /// Whether the notice can be dismissed by the user
    #[serde(default)]
    pub dismissible: bool,
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

impl Default for NoticeType {
    fn default() -> Self {
        NoticeType::Info
    }
}

impl Parameter for NoticeParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Notice
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for NoticeParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NoticeParameter({})", self.metadata.name)
    }
}

impl Displayable for NoticeParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

// Notice parameters only display information, they don't have user-editable values
// They implement only ParameterType and Displayable
