use serde::{Deserialize, Serialize};

/// A single option in a select or multi-select parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    /// Machine-readable identifier.
    pub key: String,

    /// Human-readable display label.
    pub name: String,

    /// The value produced when this option is selected.
    pub value: serde_json::Value,

    /// Optional tooltip or help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether this option is shown but not selectable.
    #[serde(default)]
    pub disabled: bool,
}

impl SelectOption {
    /// Create a new enabled option with the given key, name, and value.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>, value: serde_json::Value) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            value,
            description: None,
            disabled: false,
        }
    }
}

/// Where a select parameter gets its options from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum OptionsSource {
    /// Options defined inline in the parameter schema.
    Static { options: Vec<SelectOption> },

    /// Options loaded at runtime by a named loader.
    Dynamic { loader_key: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_option() {
        let opt = SelectOption::new("us_east", "US East", serde_json::json!("us-east-1"));
        assert_eq!(opt.key, "us_east");
        assert_eq!(opt.name, "US East");
        assert_eq!(opt.value, serde_json::json!("us-east-1"));
        assert!(opt.description.is_none());
        assert!(!opt.disabled);
    }

    #[test]
    fn option_equality() {
        let a = SelectOption::new("a", "A", serde_json::json!(1));
        let b = SelectOption::new("a", "A", serde_json::json!(1));
        assert_eq!(a, b);

        let c = SelectOption::new("a", "A", serde_json::json!(2));
        assert_ne!(a, c);
    }

    #[test]
    fn serde_option_round_trip() {
        let opt = SelectOption {
            key: "json".into(),
            name: "JSON".into(),
            value: serde_json::json!("application/json"),
            description: Some("JSON format".into()),
            disabled: false,
        };

        let json = serde_json::to_string(&opt).unwrap();
        let deserialized: SelectOption = serde_json::from_str(&json).unwrap();
        assert_eq!(opt, deserialized);
    }

    #[test]
    fn disabled_option() {
        let opt = SelectOption {
            key: "beta".into(),
            name: "Beta Feature".into(),
            value: serde_json::json!("beta"),
            description: Some("Not yet available".into()),
            disabled: true,
        };

        let json = serde_json::to_string(&opt).unwrap();
        assert!(json.contains("\"disabled\":true"));
    }

    #[test]
    fn serde_static_source() {
        let source = OptionsSource::Static {
            options: vec![
                SelectOption::new("a", "Alpha", serde_json::json!("a")),
                SelectOption::new("b", "Bravo", serde_json::json!("b")),
            ],
        };

        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"source\":\"static\""));

        let deserialized: OptionsSource = serde_json::from_str(&json).unwrap();
        match deserialized {
            OptionsSource::Static { options } => assert_eq!(options.len(), 2),
            _ => panic!("expected Static"),
        }
    }

    #[test]
    fn serde_dynamic_source() {
        let source = OptionsSource::Dynamic {
            loader_key: "load_regions".into(),
        };

        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"source\":\"dynamic\""));
        assert!(json.contains("\"loader_key\":\"load_regions\""));

        let deserialized: OptionsSource = serde_json::from_str(&json).unwrap();
        match deserialized {
            OptionsSource::Dynamic { loader_key } => assert_eq!(loader_key, "load_regions"),
            _ => panic!("expected Dynamic"),
        }
    }

    #[test]
    fn optional_fields_omitted_from_json() {
        let opt = SelectOption::new("k", "K", serde_json::json!(1));
        let json = serde_json::to_string(&opt).unwrap();
        assert!(!json.contains("description"));
    }
}
