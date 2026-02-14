use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to datetime parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DateTimeOptions {
    /// Earliest allowed datetime (as ISO 8601 string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,

    /// Latest allowed datetime (as ISO 8601 string).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,

    /// Display format string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

/// A combined date and time picker parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DateTimeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<DateTimeOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl DateTimeParameter {
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
    fn new_creates_minimal_datetime() {
        let p = DateTimeParameter::new("scheduled_at", "Scheduled At");
        assert_eq!(p.metadata.key, "scheduled_at");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = DateTimeParameter {
            metadata: ParameterMetadata::new("start", "Start Time"),
            default: Some("2026-01-01T00:00:00Z".into()),
            options: Some(DateTimeOptions {
                min: Some("2020-01-01T00:00:00Z".into()),
                max: Some("2030-12-31T23:59:59Z".into()),
                format: Some("%Y-%m-%dT%H:%M:%S".into()),
            }),
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: DateTimeParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "start");
        assert!(deserialized.options.is_some());
    }
}
