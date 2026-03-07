//! Generic List parameter for repeatable items.

use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::list::ListOptions;
use crate::validation::ValidationRule;

/// A repeatable list of items sharing the same template.
///
/// Use case: HTTP headers, email recipients, environment variables.
///
/// ## Example
///
/// ```
/// use nebula_parameter::typed::{List, Text, Plain};
///
/// let headers = List::builder("headers", Text::<Plain>::builder("header")
///     .label("Header Value")
///     .build()
///     .into())
///     .label("HTTP Headers")
///     .min_items(1)
///     .max_items(10)
///     .add_button_label("Add Header")
///     .sortable(true)
///     .build();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct List {
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

impl List {
    /// Create a new list parameter builder.
    #[must_use]
    pub fn builder(key: impl Into<String>, item_template: ParameterDef) -> ListBuilder {
        ListBuilder::new(key, item_template)
    }

    /// Create a minimal list parameter.
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

/// Builder for List parameters.
#[derive(Debug)]
pub struct ListBuilder {
    metadata: ParameterMetadata,
    item_template: Box<ParameterDef>,
    default: Option<Vec<serde_json::Value>>,
    options: Option<ListOptions>,
    display: Option<ParameterDisplay>,
    validation: Vec<ValidationRule>,
}

impl ListBuilder {
    fn new(key: impl Into<String>, item_template: ParameterDef) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            item_template: Box::new(item_template),
            default: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }

    /// Set the display label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.metadata.name = label.into();
        self
    }

    /// Set the description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.metadata.description = Some(desc.into());
        self
    }

    /// Mark as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.metadata.required = true;
        self
    }

    /// Set default list values.
    #[must_use]
    pub fn default_values(mut self, values: Vec<serde_json::Value>) -> Self {
        self.default = Some(values);
        self
    }

    /// Set minimum number of items.
    #[must_use]
    pub fn min_items(mut self, min: usize) -> Self {
        self.options
            .get_or_insert_with(ListOptions::default)
            .min_items = Some(min);
        self
    }

    /// Set maximum number of items.
    #[must_use]
    pub fn max_items(mut self, max: usize) -> Self {
        self.options
            .get_or_insert_with(ListOptions::default)
            .max_items = Some(max);
        self
    }

    /// Set label for the add button.
    #[must_use]
    pub fn add_button_label(mut self, label: impl Into<String>) -> Self {
        self.options
            .get_or_insert_with(ListOptions::default)
            .add_button_label = Some(label.into());
        self
    }

    /// Enable drag-and-drop reordering.
    #[must_use]
    pub fn sortable(mut self, sortable: bool) -> Self {
        self.options
            .get_or_insert_with(ListOptions::default)
            .sortable = sortable;
        self
    }

    /// Add a validation rule.
    #[must_use]
    pub fn validation(mut self, rule: ValidationRule) -> Self {
        self.validation.push(rule);
        self
    }

    /// Build the List parameter.
    #[must_use]
    pub fn build(self) -> List {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        List {
            metadata,
            item_template: self.item_template,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::text::TextParameter;

    #[test]
    fn builder_creates_list() {
        let list = List::builder(
            "items",
            ParameterDef::Text(TextParameter::new("item", "Item")),
        )
        .label("Items List")
        .min_items(1)
        .max_items(5)
        .sortable(true)
        .required()
        .build();

        assert_eq!(list.metadata.key, "items");
        assert_eq!(list.metadata.name, "Items List");
        assert!(list.metadata.required);
        assert_eq!(list.options.as_ref().unwrap().min_items, Some(1));
        assert_eq!(list.options.as_ref().unwrap().sortable, true);
    }
}
