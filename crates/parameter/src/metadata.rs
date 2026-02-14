use serde::{Deserialize, Serialize};

/// Descriptive metadata attached to every parameter definition.
///
/// This is the human-facing information: labels, hints, placeholders.
/// It is separate from the parameter's type and value semantics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterMetadata {
    /// Unique key identifying this parameter within its parent scope.
    pub key: String,

    /// Human-readable display name.
    pub name: String,

    /// Longer description shown as tooltip or help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether the user must provide a value.
    #[serde(default)]
    pub required: bool,

    /// Placeholder text shown in empty input fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// Short contextual hint displayed near the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,

    /// Whether the value should be masked in the UI and logs.
    #[serde(default)]
    pub sensitive: bool,
}

impl ParameterMetadata {
    /// Create metadata with the required key and display name.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_key_and_name() {
        let meta = ParameterMetadata::new("api_key", "API Key");
        assert_eq!(meta.key, "api_key");
        assert_eq!(meta.name, "API Key");
        assert!(!meta.required);
        assert!(!meta.sensitive);
        assert!(meta.description.is_none());
        assert!(meta.placeholder.is_none());
        assert!(meta.hint.is_none());
    }

    #[test]
    fn default_is_empty() {
        let meta = ParameterMetadata::default();
        assert!(meta.key.is_empty());
        assert!(meta.name.is_empty());
        assert!(!meta.required);
        assert!(!meta.sensitive);
    }

    #[test]
    fn serde_round_trip_minimal() {
        let meta = ParameterMetadata::new("host", "Hostname");
        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: ParameterMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.key, "host");
        assert_eq!(deserialized.name, "Hostname");
        assert!(!deserialized.required);
    }

    #[test]
    fn serde_round_trip_full() {
        let meta = ParameterMetadata {
            key: "password".into(),
            name: "Password".into(),
            description: Some("Your account password".into()),
            required: true,
            placeholder: Some("Enter password...".into()),
            hint: Some("At least 8 characters".into()),
            sensitive: true,
        };

        let json = serde_json::to_string_pretty(&meta).unwrap();
        let deserialized: ParameterMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.key, "password");
        assert_eq!(deserialized.name, "Password");
        assert_eq!(
            deserialized.description.as_deref(),
            Some("Your account password")
        );
        assert!(deserialized.required);
        assert_eq!(
            deserialized.placeholder.as_deref(),
            Some("Enter password...")
        );
        assert_eq!(deserialized.hint.as_deref(), Some("At least 8 characters"));
        assert!(deserialized.sensitive);
    }

    #[test]
    fn optional_fields_omitted_from_json() {
        let meta = ParameterMetadata::new("name", "Name");
        let json = serde_json::to_string(&meta).unwrap();

        assert!(!json.contains("description"));
        assert!(!json.contains("placeholder"));
        assert!(!json.contains("hint"));
    }

    #[test]
    fn deserialize_with_missing_optional_fields() {
        let json = r#"{"key": "count", "name": "Count"}"#;
        let meta: ParameterMetadata = serde_json::from_str(json).unwrap();

        assert_eq!(meta.key, "count");
        assert_eq!(meta.name, "Count");
        assert!(!meta.required);
        assert!(!meta.sensitive);
        assert!(meta.description.is_none());
    }
}
