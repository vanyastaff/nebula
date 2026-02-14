use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to number parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumberOptions {
    /// Minimum allowed value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,

    /// Maximum allowed value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,

    /// Step increment for UI spinners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,

    /// Number of decimal places to display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
}

/// A numeric input parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumberParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<f64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<NumberOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl NumberParameter {
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
    fn new_creates_minimal_number() {
        let p = NumberParameter::new("port", "Port");
        assert_eq!(p.metadata.key, "port");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = NumberParameter {
            metadata: ParameterMetadata::new("timeout", "Timeout (s)"),
            default: Some(30.0),
            options: Some(NumberOptions {
                min: Some(1.0),
                max: Some(300.0),
                step: Some(1.0),
                precision: Some(0),
            }),
            display: None,
            validation: vec![ValidationRule::min(1.0), ValidationRule::max(300.0)],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: NumberParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "timeout");
        assert_eq!(deserialized.default, Some(30.0));
        assert_eq!(deserialized.validation.len(), 2);
    }
}
