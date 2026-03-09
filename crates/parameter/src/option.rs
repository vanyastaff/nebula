use serde::{Deserialize, Serialize};

/// A single option in a select or multi-select parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    /// Legacy machine-readable identifier kept for migration compatibility.
    ///
    /// Canonical v2 wire shape does not emit this field.
    #[serde(default, skip_serializing)]
    pub key: String,

    /// The value produced when this option is selected.
    pub value: serde_json::Value,

    /// Human-readable display label.
    #[serde(rename = "label", alias = "name")]
    pub label: String,

    /// Optional tooltip or help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether this option is shown but not selectable.
    #[serde(default)]
    pub disabled: bool,
}

impl SelectOption {
    /// Creates a new enabled option with canonical v2 fields.
    #[must_use]
    pub fn new(value: serde_json::Value, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            key: String::new(),
            description: None,
            disabled: false,
        }
    }

    /// Creates an option from legacy `key` + `name` + `value` shape.
    #[must_use]
    pub fn with_key(
        key: impl Into<String>,
        name: impl Into<String>,
        value: serde_json::Value,
    ) -> Self {
        Self {
            key: key.into(),
            value,
            label: name.into(),
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
        #[serde(alias = "loader_key")]
        provider: String,
        /// Re-resolve options when these sibling fields change.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
    },
}

/// Backward-compatible alias for the canonical [`OptionSource`] type.
pub type OptionsSource = OptionSource;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_option() {
        let opt = SelectOption::new(serde_json::json!("us-east-1"), "US East");
        assert!(opt.key.is_empty());
        assert_eq!(opt.label, "US East");
        assert_eq!(opt.value, serde_json::json!("us-east-1"));
        assert!(opt.description.is_none());
        assert!(!opt.disabled);
    }

    #[test]
    fn legacy_constructor_keeps_key() {
        let opt = SelectOption::with_key("us_east", "US East", serde_json::json!("us-east-1"));
        assert_eq!(opt.key, "us_east");
        assert_eq!(opt.label, "US East");
    }

    #[test]
    fn option_equality() {
        let a = SelectOption::with_key("a", "A", serde_json::json!(1));
        let b = SelectOption::with_key("a", "A", serde_json::json!(1));
        assert_eq!(a, b);

        let c = SelectOption::with_key("a", "A", serde_json::json!(2));
        assert_ne!(a, c);
    }

    #[test]
    fn serde_option_round_trip() {
        let opt = SelectOption {
            key: "json".into(),
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
        assert!(deserialized.key.is_empty());
    }

    #[test]
    fn disabled_option() {
        let opt = SelectOption {
            key: "beta".into(),
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
    fn legacy_name_and_loader_key_still_deserialize() {
        let option_json = serde_json::json!({
            "key": "legacy",
            "name": "Legacy",
            "value": "legacy"
        });
        let option: SelectOption = serde_json::from_value(option_json).unwrap();
        assert_eq!(option.label, "Legacy");
        assert_eq!(option.value, serde_json::json!("legacy"));

        let source_json = serde_json::json!({
            "source": "dynamic",
            "loader_key": "load_legacy"
        });
        let source: OptionSource = serde_json::from_value(source_json).unwrap();
        match source {
            OptionSource::Dynamic { provider, .. } => assert_eq!(provider, "load_legacy"),
            _ => panic!("expected dynamic source"),
        }
    }

    #[test]
    fn optional_fields_omitted_from_json() {
        let opt = SelectOption::new(serde_json::json!(1), "K");
        let json = serde_json::to_string(&opt).unwrap();
        assert!(!json.contains("description"));
        assert!(!json.contains("key"));
    }
}
