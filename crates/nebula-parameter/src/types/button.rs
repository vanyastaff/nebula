use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{Displayable, ParameterDisplay, ParameterKind, ParameterMetadata, ParameterType};

/// Parameter for button actions
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
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

impl Default for ButtonType {
    fn default() -> Self {
        ButtonType::Primary
    }
}

impl ParameterType for ButtonParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Button
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for ButtonParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ButtonParameter({})", self.metadata.name)
    }
}

impl Displayable for ButtonParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

// Button parameters are interactive but don't have values
// They implement only ParameterType and Displayable
