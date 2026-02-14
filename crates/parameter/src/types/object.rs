use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to object parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectOptions {
    /// Whether the object can be collapsed in the UI.
    #[serde(default)]
    pub collapsible: bool,

    /// Whether the object starts collapsed.
    #[serde(default)]
    pub collapsed_by_default: bool,
}

/// A fixed set of named child parameters grouped as an object.
///
/// Use case: DB connection (host + port + user + password grouped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The child parameters that make up this object.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ParameterDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ObjectOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl ObjectParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            fields: Vec::new(),
            default: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }

    /// Add a child field (builder-style).
    #[must_use]
    pub fn with_field(mut self, field: ParameterDef) -> Self {
        self.fields.push(field);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NumberParameter;
    use crate::types::TextParameter;
    use serde_json::json;

    #[test]
    fn new_creates_minimal_object() {
        let p = ObjectParameter::new("connection", "Connection");
        assert_eq!(p.metadata.key, "connection");
        assert_eq!(p.metadata.name, "Connection");
        assert!(p.fields.is_empty());
        assert!(p.default.is_none());
        assert!(p.options.is_none());
        assert!(p.display.is_none());
        assert!(p.validation.is_empty());
    }

    #[test]
    fn with_field_chains() {
        let p = ObjectParameter::new("db", "Database")
            .with_field(ParameterDef::Text(TextParameter::new("host", "Host")))
            .with_field(ParameterDef::Number(NumberParameter::new("port", "Port")));

        assert_eq!(p.fields.len(), 2);
        assert_eq!(p.fields[0].key(), "host");
        assert_eq!(p.fields[1].key(), "port");
    }

    #[test]
    fn serde_round_trip() {
        let p = ObjectParameter::new("conn", "Connection")
            .with_field(ParameterDef::Text(TextParameter::new("host", "Host")));

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: ObjectParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "conn");
        assert_eq!(deserialized.fields.len(), 1);
        assert_eq!(deserialized.fields[0].key(), "host");
    }

    #[test]
    fn serde_with_default_and_options() {
        let p = ObjectParameter {
            metadata: ParameterMetadata::new("settings", "Settings"),
            fields: Vec::new(),
            default: Some(json!({"timeout": 30})),
            options: Some(ObjectOptions {
                collapsible: true,
                collapsed_by_default: true,
            }),
            display: None,
            validation: Vec::new(),
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: ObjectParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.default, Some(json!({"timeout": 30})));
        let opts = deserialized.options.unwrap();
        assert!(opts.collapsible);
        assert!(opts.collapsed_by_default);
    }
}
