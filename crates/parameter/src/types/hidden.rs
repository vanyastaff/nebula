use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// A hidden parameter that carries a value but is not shown in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiddenParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl HiddenParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_creates_minimal_hidden() {
        let p = HiddenParameter::new("internal_id", "Internal ID");
        assert_eq!(p.metadata.key, "internal_id");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = HiddenParameter {
            metadata: ParameterMetadata::new("node_version", "Node Version"),
            default: Some(json!(2)),
            display: None,
            validation: vec![],
        };

        let json_str = serde_json::to_string(&p).unwrap();
        let deserialized: HiddenParameter = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.metadata.key, "node_version");
        assert_eq!(deserialized.default, Some(json!(2)));
    }
}
