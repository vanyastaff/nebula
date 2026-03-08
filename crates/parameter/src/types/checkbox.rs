use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::subtype::BooleanSubtype;
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

    /// Semantic boolean subtype.
    #[serde(default, skip_serializing_if = "is_default_boolean_subtype")]
    pub subtype: BooleanSubtype,

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
            subtype: BooleanSubtype::default(),
            display: None,
            validation: Vec::new(),
        }
    }

    /// Sets semantic subtype for this checkbox parameter.
    #[must_use]
    pub fn subtype(mut self, subtype: BooleanSubtype) -> Self {
        self.subtype = subtype;
        self
    }

    /// Creates a feature-flag checkbox.
    #[must_use]
    pub fn feature_flag(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name).subtype(BooleanSubtype::FeatureFlag)
    }

    /// Creates a consent checkbox.
    #[must_use]
    pub fn consent(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name).subtype(BooleanSubtype::Consent)
    }
}

fn is_default_boolean_subtype(subtype: &BooleanSubtype) -> bool {
    *subtype == BooleanSubtype::default()
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
            subtype: BooleanSubtype::Toggle,
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: CheckboxParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "debug");
        assert_eq!(deserialized.default, Some(false));
    }
}
