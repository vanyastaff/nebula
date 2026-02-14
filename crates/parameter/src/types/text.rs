use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to text parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextOptions {
    /// Regex pattern the value must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,

    /// Maximum allowed character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,

    /// Minimum required character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
}

/// A single-line text input parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TextOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl TextParameter {
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
    fn new_creates_minimal_text() {
        let p = TextParameter::new("username", "Username");
        assert_eq!(p.metadata.key, "username");
        assert_eq!(p.metadata.name, "Username");
        assert!(p.default.is_none());
        assert!(p.options.is_none());
        assert!(p.display.is_none());
        assert!(p.validation.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let p = TextParameter {
            metadata: ParameterMetadata::new("email", "Email"),
            default: Some("user@example.com".into()),
            options: Some(TextOptions {
                pattern: Some(r"^.+@.+\..+$".into()),
                max_length: Some(255),
                min_length: Some(5),
            }),
            display: None,
            validation: vec![ValidationRule::min_length(5)],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: TextParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "email");
        assert_eq!(deserialized.default.as_deref(), Some("user@example.com"));
        assert!(deserialized.options.is_some());
        assert_eq!(deserialized.validation.len(), 1);
    }
}
