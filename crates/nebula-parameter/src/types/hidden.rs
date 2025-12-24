use serde::{Deserialize, Serialize};

use crate::core::{Parameter, ParameterKind, ParameterMetadata};

/// Parameter that is hidden from the user interface but can store values
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct HiddenParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<String>,
}

impl Parameter for HiddenParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Hidden
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for HiddenParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HiddenParameter({})", self.metadata.name)
    }
}

// Hidden parameters don't implement Validatable or Displayable by design
// They're meant to be internal-only values
