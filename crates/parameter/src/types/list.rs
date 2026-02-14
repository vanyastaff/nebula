use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Options specific to list parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListOptions {
    /// Minimum number of items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<usize>,

    /// Maximum number of items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<usize>,

    /// Label for the add button in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_button_label: Option<String>,

    /// Whether items can be reordered by dragging.
    #[serde(default)]
    pub sortable: bool,
}

/// A repeatable list of items sharing the same template.
///
/// Use case: HTTP headers, email recipients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Shape of each item in the list (boxed to avoid infinite size).
    pub item_template: Box<ParameterDef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Vec<serde_json::Value>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ListOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl ListParameter {
    /// Create a new list parameter. The item template is required.
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        item_template: ParameterDef,
    ) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            item_template: Box::new(item_template),
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
    use crate::types::TextParameter;
    use serde_json::json;

    #[test]
    fn new_creates_minimal_list() {
        let template = ParameterDef::Text(TextParameter::new("header", "Header"));
        let p = ListParameter::new("headers", "Headers", template);
        assert_eq!(p.metadata.key, "headers");
        assert_eq!(p.metadata.name, "Headers");
        assert_eq!(p.item_template.key(), "header");
        assert!(p.default.is_none());
        assert!(p.options.is_none());
        assert!(p.display.is_none());
        assert!(p.validation.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let template = ParameterDef::Text(TextParameter::new("email", "Email"));
        let p = ListParameter {
            metadata: ParameterMetadata::new("recipients", "Recipients"),
            item_template: Box::new(template),
            default: Some(vec![json!("admin@example.com")]),
            options: Some(ListOptions {
                min_items: Some(1),
                max_items: Some(10),
                add_button_label: Some("Add recipient".into()),
                sortable: true,
            }),
            display: None,
            validation: vec![ValidationRule::min_items(1)],
        };

        let json_str = serde_json::to_string(&p).unwrap();
        let deserialized: ListParameter = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.metadata.key, "recipients");
        assert_eq!(deserialized.item_template.key(), "email");
        assert_eq!(deserialized.default.as_ref().unwrap().len(), 1);
        let opts = deserialized.options.unwrap();
        assert_eq!(opts.min_items, Some(1));
        assert_eq!(opts.max_items, Some(10));
        assert!(opts.sortable);
        assert_eq!(deserialized.validation.len(), 1);
    }
}
