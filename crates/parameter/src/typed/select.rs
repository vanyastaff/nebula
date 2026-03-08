//! Generic Select parameter with type-safe options.
//!
//! Unlike text/number/checkbox which use subtypes, Select is a distinct parameter
//! type with unique semantics (option loading, selection logic).

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::option::SelectOption;
use crate::types::select::SelectOptions;
use crate::validation::ValidationRule;

/// A single-choice dropdown parameter with type-safe value.
///
/// ## Example
///
/// ```
/// use nebula_parameter::typed::Select;
/// use nebula_parameter::option::SelectOption;
/// use serde_json::json;
///
/// let select = Select::builder("region")
///     .label("AWS Region")
///     .option(SelectOption::new("us-east-1", "US East", json!("us-east-1")))
///     .option(SelectOption::new("eu-west-1", "EU West", json!("eu-west-1")))
///     .default_value(json!("us-east-1"))
///     .build();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Select {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    /// The available choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select_options: Option<SelectOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Select {
    /// Create a new select parameter builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> SelectBuilder {
        SelectBuilder::new(key)
    }

    /// Create a minimal select parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: Vec::new(),
            select_options: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

/// Builder for Select parameters.
#[derive(Debug)]
pub struct SelectBuilder {
    metadata: ParameterMetadata,
    default: Option<serde_json::Value>,
    options: Vec<SelectOption>,
    select_options: Option<SelectOptions>,
    display: Option<ParameterDisplay>,
    validation: Vec<ValidationRule>,
}

impl SelectBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            default: None,
            options: Vec::new(),
            select_options: None,
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

    /// Set default value.
    #[must_use]
    pub fn default_value(mut self, value: serde_json::Value) -> Self {
        self.default = Some(value);
        self
    }

    /// Add a select option.
    #[must_use]
    pub fn option(mut self, opt: SelectOption) -> Self {
        self.options.push(opt);
        self
    }

    /// Set multiple options at once.
    #[must_use]
    pub fn options(mut self, opts: impl IntoIterator<Item = SelectOption>) -> Self {
        self.options.extend(opts);
        self
    }

    /// Set placeholder text.
    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.select_options
            .get_or_insert_with(SelectOptions::default)
            .placeholder = Some(text.into());
        self
    }

    /// Add a validation rule.
    #[must_use]
    pub fn validation(mut self, rule: ValidationRule) -> Self {
        self.validation.push(rule);
        self
    }

    /// Build the Select parameter.
    #[must_use]
    pub fn build(self) -> Select {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Select {
            metadata,
            default: self.default,
            options: self.options,
            select_options: self.select_options,
            display: self.display,
            validation: self.validation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builder_creates_select() {
        let select = Select::builder("region")
            .label("Region")
            .option(SelectOption::new("us", "US", json!("us")))
            .option(SelectOption::new("eu", "EU", json!("eu")))
            .default_value(json!("us"))
            .placeholder("Choose region...")
            .required()
            .build();

        assert_eq!(select.metadata.key, "region");
        assert_eq!(select.metadata.name, "Region");
        assert_eq!(select.options.len(), 2);
        assert_eq!(select.default, Some(json!("us")));
        assert!(select.metadata.required);
    }

    #[test]
    fn serde_round_trip() {
        let select = Select::builder("format")
            .label("Output Format")
            .option(SelectOption::new("json", "JSON", json!("json")))
            .option(SelectOption::new("xml", "XML", json!("xml")))
            .build();

        let json = serde_json::to_string(&select).unwrap();
        let deserialized: Select = serde_json::from_str(&json).unwrap();
        assert_eq!(select, deserialized);
    }
}
