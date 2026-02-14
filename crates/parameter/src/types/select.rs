use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::option::SelectOption;
use crate::validation::ValidationRule;

/// Options specific to select parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOptions {
    /// Placeholder text shown when no option is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// A single-choice dropdown parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// The available choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_options: Option<SelectOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl SelectParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: Vec::new(),
            select_options: None,
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
    fn new_creates_minimal_select() {
        let p = SelectParameter::new("region", "Region");
        assert_eq!(p.metadata.key, "region");
        assert!(p.options.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let p = SelectParameter {
            metadata: ParameterMetadata::new("format", "Output Format"),
            default: Some(json!("json")),
            options: vec![
                SelectOption::new("json", "JSON", json!("json")),
                SelectOption::new("xml", "XML", json!("xml")),
            ],
            select_options: Some(SelectOptions {
                placeholder: Some("Choose format...".into()),
            }),
            display: None,
            validation: vec![],
        };

        let json_str = serde_json::to_string(&p).unwrap();
        let deserialized: SelectParameter = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.metadata.key, "format");
        assert_eq!(deserialized.options.len(), 2);
    }
}
