use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to secret parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretOptions {
    /// Minimum required character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    /// Maximum allowed character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

/// A masked text input for sensitive values like passwords and API keys.
///
/// Always sets `metadata.sensitive = true`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SecretOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl SecretParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        let mut metadata = ParameterMetadata::new(key, name);
        metadata.sensitive = true;
        Self {
            metadata,
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
    fn new_sets_sensitive_true() {
        let p = SecretParameter::new("api_key", "API Key");
        assert_eq!(p.metadata.key, "api_key");
        assert!(p.metadata.sensitive);
    }

    #[test]
    fn serde_round_trip() {
        let p = SecretParameter::new("password", "Password");
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: SecretParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "password");
        assert!(deserialized.metadata.sensitive);
    }
}
