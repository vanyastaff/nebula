use serde::{Deserialize, Serialize};

/// A single option in a select or multi-select parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    /// The value produced when this option is selected.
    pub value: serde_json::Value,

    /// Human-readable display label.
    pub label: String,

    /// Optional tooltip or help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether this option is shown but not selectable.
    #[serde(default)]
    pub disabled: bool,
}

impl SelectOption {
    /// Creates a new enabled option.
    #[must_use]
    pub fn new(value: serde_json::Value, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            description: None,
            disabled: false,
        }
    }
}

/// Where a select parameter gets its options from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum OptionSource {
    /// Options defined inline in the parameter schema.
    Static { options: Vec<SelectOption> },

    /// Options loaded at runtime by a named provider.
    Dynamic {
        /// Provider key resolved by runtime registry.
        provider: String,
        /// Re-resolve options when these sibling fields change.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_option() {
        let opt = SelectOption::new(serde_json::json!("us-east-1"), "US East");
        assert_eq!(opt.label, "US East");
        assert_eq!(opt.value, serde_json::json!("us-east-1"));
        assert!(opt.description.is_none());
        assert!(!opt.disabled);
    }

    #[test]
    fn option_equality() {
        let a = SelectOption::new(serde_json::json!(1), "A");
        let b = SelectOption::new(serde_json::json!(1), "A");
        assert_eq!(a, b);

        let c = SelectOption::new(serde_json::json!(2), "A");
        assert_ne!(a, c);
    }

    #[test]
    fn serde_option_round_trip() {
        let opt = SelectOption {
            value: serde_json::json!("application/json"),
            label: "JSON".into(),
            description: Some("JSON format".into()),
            disabled: false,
        };

        let json = serde_json::to_string(&opt).unwrap();
        let deserialized: SelectOption = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.value, opt.value);
        assert_eq!(deserialized.label, opt.label);
        assert_eq!(deserialized.description, opt.description);
        assert_eq!(deserialized.disabled, opt.disabled);
    }

    #[test]
    fn disabled_option() {
        let opt = SelectOption {
            value: serde_json::json!("beta"),
            label: "Beta Feature".into(),
            description: Some("Not yet available".into()),
            disabled: true,
        };

        let json = serde_json::to_string(&opt).unwrap();
        assert!(json.contains("\"disabled\":true"));
    }

    #[test]
    fn serde_static_source() {
        let source = OptionSource::Static {
            options: vec![
                SelectOption::new(serde_json::json!("a"), "Alpha"),
                SelectOption::new(serde_json::json!("b"), "Bravo"),
            ],
        };

        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"source\":\"static\""));

        let deserialized: OptionSource = serde_json::from_str(&json).unwrap();
        match deserialized {
            OptionSource::Static { options } => assert_eq!(options.len(), 2),
            _ => panic!("expected Static"),
        }
    }

    #[test]
    fn serde_dynamic_source() {
        let source = OptionSource::Dynamic {
            provider: "load_regions".into(),
            depends_on: Vec::new(),
        };

        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"source\":\"dynamic\""));
        assert!(json.contains("\"provider\":\"load_regions\""));

        let deserialized: OptionSource = serde_json::from_str(&json).unwrap();
        match deserialized {
            OptionSource::Dynamic {
                provider,
                depends_on,
            } => {
                assert_eq!(provider, "load_regions");
                assert!(depends_on.is_empty());
            }
            _ => panic!("expected Dynamic"),
        }
    }

    #[test]
    fn optional_fields_omitted_from_json() {
        let opt = SelectOption::new(serde_json::json!(1), "K");
        let json = serde_json::to_string(&opt).unwrap();
        assert!(!json.contains("description"));
    }
}
