use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::option::SelectOption;
use crate::validation::ValidationRule;

/// Options specific to multi-select parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiSelectOptions {
    /// Minimum number of selections required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<usize>,

    /// Maximum number of selections allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<usize>,
}

/// A multi-choice selection parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiSelectParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Vec<serde_json::Value>>,

    /// The available choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_select_options: Option<MultiSelectOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl MultiSelectParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: Vec::new(),
            multi_select_options: None,
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
    fn new_creates_minimal_multi_select() {
        let p = MultiSelectParameter::new("tags", "Tags");
        assert_eq!(p.metadata.key, "tags");
        assert!(p.options.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let p = MultiSelectParameter {
            metadata: ParameterMetadata::new("features", "Features"),
            default: Some(vec![json!("logging"), json!("metrics")]),
            options: vec![
                SelectOption::new("logging", "Logging", json!("logging")),
                SelectOption::new("metrics", "Metrics", json!("metrics")),
                SelectOption::new("tracing", "Tracing", json!("tracing")),
            ],
            multi_select_options: Some(MultiSelectOptions {
                min_selections: Some(1),
                max_selections: Some(3),
            }),
            display: None,
            validation: vec![],
        };

        let json_str = serde_json::to_string(&p).unwrap();
        let deserialized: MultiSelectParameter = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.metadata.key, "features");
        assert_eq!(deserialized.options.len(), 3);
        assert_eq!(deserialized.default.as_ref().unwrap().len(), 2);
    }
}
