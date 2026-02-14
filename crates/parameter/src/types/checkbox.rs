use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to checkbox parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckboxOptions {
    /// Label displayed next to the checkbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Additional help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

/// A boolean toggle parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckboxParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<CheckboxOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl CheckboxParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_minimal_checkbox() {
        let p = CheckboxParameter::new("enabled", "Enabled");
        assert_eq!(p.metadata.key, "enabled");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = CheckboxParameter {
            metadata: ParameterMetadata::new("debug", "Debug Mode"),
            default: Some(false),
            options: Some(CheckboxOptions {
                label: Some("Enable debug logging".into()),
                help_text: Some("Verbose output".into()),
            }),
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: CheckboxParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "debug");
        assert_eq!(deserialized.default, Some(false));
    }
}
