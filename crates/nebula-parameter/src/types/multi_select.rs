use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, SelectOption, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for selecting multiple options from a dropdown
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct MultiSelectParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<Vec<String>>,

    /// Available options for selection
    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub multi_select_options: Option<MultiSelectParameterOptions>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct MultiSelectParameterOptions {
    /// Minimum number of selections required
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<usize>,

    /// Maximum number of selections allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<usize>,
}

impl Describable for MultiSelectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::MultiSelect
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for MultiSelectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MultiSelectParameter({})", self.base.metadata.name)
    }
}

impl Validatable for MultiSelectParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Array)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_array().is_some_and(|arr| arr.is_empty())
    }
}

impl Displayable for MultiSelectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}

impl MultiSelectParameter {
    /// Get option by value
    #[must_use]
    pub fn get_option_by_value(&self, value: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// Get option by key
    #[must_use]
    pub fn get_option_by_key(&self, key: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.key == key)
    }

    /// Get display names for given selections
    #[must_use]
    pub fn get_display_names(&self, selections: &[String]) -> Vec<String> {
        selections
            .iter()
            .filter_map(|value| {
                self.get_option_by_value(value)
                    .map(|option| option.name.clone())
                    .or_else(|| Some(value.clone())) // Fallback to raw value
            })
            .collect()
    }
}
