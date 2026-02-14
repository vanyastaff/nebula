use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to time parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeOptions {
    /// Earliest allowed time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,

    /// Latest allowed time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,

    /// Display format string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// Whether to use 24-hour format in the UI.
    #[serde(default)]
    pub use_24h: bool,
}

/// A time-only picker parameter (no date component).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TimeOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl TimeParameter {
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
    fn new_creates_minimal_time() {
        let p = TimeParameter::new("alarm", "Alarm Time");
        assert_eq!(p.metadata.key, "alarm");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = TimeParameter {
            metadata: ParameterMetadata::new("start_time", "Start Time"),
            default: Some("09:00".into()),
            options: Some(TimeOptions {
                min: Some("08:00".into()),
                max: Some("18:00".into()),
                format: None,
                use_24h: true,
            }),
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: TimeParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "start_time");
        assert!(deserialized.options.as_ref().unwrap().use_24h);
    }
}
