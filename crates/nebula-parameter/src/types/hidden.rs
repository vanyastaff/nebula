use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterKind, ParameterMetadata,
    Validatable,
};
use nebula_value::Value;

/// Parameter that is hidden from the user interface but can store values
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct HiddenParameter {
    /// Base parameter fields (metadata, display, validation)
    /// Note: display and validation are ignored for hidden parameters
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<String>,
}

impl Describable for HiddenParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Hidden
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for HiddenParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HiddenParameter({})", self.base.metadata.name)
    }
}

// Hidden parameters implement minimal Validatable and Displayable for blanket Parameter impl
impl Validatable for HiddenParameter {
    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().map(|s| s.is_empty()).unwrap_or(false)
    }
}

impl Displayable for HiddenParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        None // Hidden parameters are never displayed
    }

    fn set_display(&mut self, _display: Option<ParameterDisplay>) {
        // No-op for hidden parameters
    }
}
