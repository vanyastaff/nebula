//! Generic MultiSelect parameter.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::option::SelectOption;
use crate::types::multi_select::MultiSelectOptions;
use crate::validation::ValidationRule;

/// A multi-choice selection parameter.
///
/// ## Example
///
/// ```
/// use nebula_parameter::typed::MultiSelect;
/// use nebula_parameter::option::SelectOption;
/// use serde_json::json;
///
/// let tags = MultiSelect::builder("tags")
///     .label("Tags")
///     .option(SelectOption::new("prod", "Production", json!("prod")))
///     .option(SelectOption::new("staging", "Staging", json!("staging")))
///     .min_selections(1)
///     .max_selections(3)
///     .build();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiSelect {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Vec<serde_json::Value>>,

    /// The available choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_select_options: Option<MultiSelectOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl MultiSelect {
    /// Create a new multi-select parameter builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> MultiSelectBuilder {
        MultiSelectBuilder::new(key)
    }

    /// Create a minimal multi-select parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: Vec::new(),
            multi_select_options: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

/// Builder for MultiSelect parameters.
#[derive(Debug)]
pub struct MultiSelectBuilder {
    metadata: ParameterMetadata,
    default: Option<Vec<serde_json::Value>>,
    options: Vec<SelectOption>,
    multi_select_options: Option<MultiSelectOptions>,
    display: Option<ParameterDisplay>,
    validation: Vec<ValidationRule>,
}

impl MultiSelectBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            default: None,
            options: Vec::new(),
            multi_select_options: None,
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

    /// Set default selected values.
    #[must_use]
    pub fn default_values(mut self, values: Vec<serde_json::Value>) -> Self {
        self.default = Some(values);
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

    /// Set minimum number of selections required.
    #[must_use]
    pub fn min_selections(mut self, min: usize) -> Self {
        self.multi_select_options
            .get_or_insert_with(MultiSelectOptions::default)
            .min_selections = Some(min);
        self
    }

    /// Set maximum number of selections allowed.
    #[must_use]
    pub fn max_selections(mut self, max: usize) -> Self {
        self.multi_select_options
            .get_or_insert_with(MultiSelectOptions::default)
            .max_selections = Some(max);
        self
    }

    /// Add a validation rule.
    #[must_use]
    pub fn validation(mut self, rule: ValidationRule) -> Self {
        self.validation.push(rule);
        self
    }

    /// Build the MultiSelect parameter.
    #[must_use]
    pub fn build(self) -> MultiSelect {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        MultiSelect {
            metadata,
            default: self.default,
            options: self.options,
            multi_select_options: self.multi_select_options,
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
    fn builder_creates_multi_select() {
        let ms = MultiSelect::builder("tags")
            .label("Tags")
            .option(SelectOption::new("a", "A", json!("a")))
            .option(SelectOption::new("b", "B", json!("b")))
            .min_selections(1)
            .max_selections(2)
            .required()
            .build();

        assert_eq!(ms.metadata.key, "tags");
        assert_eq!(ms.options.len(), 2);
        assert!(ms.metadata.required);
        assert_eq!(
            ms.multi_select_options.as_ref().unwrap().min_selections,
            Some(1)
        );
    }
}
