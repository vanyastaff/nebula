use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to textarea parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextareaOptions {
    /// Minimum required character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    /// Maximum allowed character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,

    /// Number of visible text rows in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
}

/// A multi-line text input parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextareaParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TextareaOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl TextareaParameter {
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
    fn new_creates_minimal_textarea() {
        let p = TextareaParameter::new("body", "Body");
        assert_eq!(p.metadata.key, "body");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = TextareaParameter {
            metadata: ParameterMetadata::new("notes", "Notes"),
            default: Some("Default notes".into()),
            options: Some(TextareaOptions {
                min_length: Some(10),
                max_length: Some(1000),
                rows: Some(5),
            }),
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: TextareaParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "notes");
        assert_eq!(deserialized.options.as_ref().unwrap().rows, Some(5));
    }
}
