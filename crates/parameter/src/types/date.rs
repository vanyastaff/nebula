use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to date parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateOptions {
    /// Earliest allowed date (as ISO 8601 date string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,

    /// Latest allowed date (as ISO 8601 date string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,

    /// Display format string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// A date picker parameter (no time component).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DateParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<DateOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl DateParameter {
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
    fn new_creates_minimal_date() {
        let p = DateParameter::new("birthday", "Birthday");
        assert_eq!(p.metadata.key, "birthday");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = DateParameter {
            metadata: ParameterMetadata::new("deadline", "Deadline"),
            default: Some("2026-12-31".into()),
            options: Some(DateOptions {
                min: Some("2026-01-01".into()),
                max: None,
                format: Some("%Y-%m-%d".into()),
            }),
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: DateParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "deadline");
        assert_eq!(deserialized.default.as_deref(), Some("2026-12-31"));
    }
}
