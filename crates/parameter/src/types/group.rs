use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;

/// Options specific to group parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupOptions {
    /// Whether the group can be collapsed in the UI.
    #[serde(default)]
    pub collapsible: bool,

    /// Whether the group starts collapsed.
    #[serde(default)]
    pub collapsed_by_default: bool,

    /// Whether the group has a visible border.
    #[serde(default)]
    pub bordered: bool,
}

/// A UI-only visual grouping of parameters.
///
/// Group carries no value — children's values are stored flat.
/// Use case: "Advanced Settings" collapsible section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The grouped child parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParameterDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<GroupOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,
}

impl GroupParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            parameters: Vec::new(),
            options: None,
            display: None,
        }
    }

    /// Add a child parameter (builder-style).
    #[must_use]
    pub fn with_parameter(mut self, param: ParameterDef) -> Self {
        self.parameters.push(param);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CheckboxParameter, NumberParameter};

    #[test]
    fn new_creates_minimal_group() {
        let p = GroupParameter::new("advanced", "Advanced Settings");
        assert_eq!(p.metadata.key, "advanced");
        assert_eq!(p.metadata.name, "Advanced Settings");
        assert!(p.parameters.is_empty());
        assert!(p.options.is_none());
        assert!(p.display.is_none());
    }

    #[test]
    fn with_parameter_chains() {
        let p = GroupParameter::new("advanced", "Advanced")
            .with_parameter(ParameterDef::Number(NumberParameter::new("timeout", "Timeout")))
            .with_parameter(ParameterDef::Checkbox(CheckboxParameter::new(
                "debug",
                "Debug Mode",
            )));

        assert_eq!(p.parameters.len(), 2);
        assert_eq!(p.parameters[0].key(), "timeout");
        assert_eq!(p.parameters[1].key(), "debug");
    }

    #[test]
    fn serde_round_trip() {
        let p = GroupParameter {
            metadata: ParameterMetadata::new("extra", "Extra Options"),
            parameters: vec![ParameterDef::Number(NumberParameter::new(
                "retries",
                "Retries",
            ))],
            options: Some(GroupOptions {
                collapsible: true,
                collapsed_by_default: true,
                bordered: false,
            }),
            display: None,
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: GroupParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "extra");
        assert_eq!(deserialized.parameters.len(), 1);
        let opts = deserialized.options.unwrap();
        assert!(opts.collapsible);
        assert!(opts.collapsed_by_default);
        assert!(!opts.bordered);
    }
}
