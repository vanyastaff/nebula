//! MultiSelect parameter type for selecting multiple options

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, SelectOption, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for selecting multiple options from a dropdown
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = MultiSelectParameter::builder()
///     .key("tags")
///     .name("Tags")
///     .description("Select one or more tags")
///     .options(vec![
///         SelectOption::new("bug", "Bug", "bug"),
///         SelectOption::new("feature", "Feature", "feature"),
///         SelectOption::new("docs", "Documentation", "docs"),
///     ])
///     .multi_select_options(
///         MultiSelectParameterOptions::builder()
///             .min_selections(1)
///             .max_selections(5)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiSelectParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Vec<String>>,

    /// Available options for selection
    pub options: Vec<SelectOption>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_select_options: Option<MultiSelectParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for multi-select parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MultiSelectParameterOptions {
    /// Minimum number of selections required
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<usize>,

    /// Maximum number of selections allowed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<usize>,
}

// =============================================================================
// MultiSelectParameter Builder
// =============================================================================

/// Builder for `MultiSelectParameter`
#[derive(Debug, Default)]
pub struct MultiSelectParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<Vec<String>>,
    options: Vec<SelectOption>,
    multi_select_options: Option<MultiSelectParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl MultiSelectParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> MultiSelectParameterBuilder {
        MultiSelectParameterBuilder::new()
    }
}

impl MultiSelectParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            placeholder: None,
            hint: None,
            default: None,
            options: Vec::new(),
            multi_select_options: None,
            display: None,
            validation: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata methods
    // -------------------------------------------------------------------------

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the display name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the default values
    #[must_use]
    pub fn default(mut self, default: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.default = Some(default.into_iter().map(Into::into).collect());
        self
    }

    /// Set the available options
    #[must_use]
    pub fn options(mut self, options: impl IntoIterator<Item = SelectOption>) -> Self {
        self.options = options.into_iter().collect();
        self
    }

    /// Add a single option
    #[must_use]
    pub fn option(mut self, option: SelectOption) -> Self {
        self.options.push(option);
        self
    }

    /// Set multi-select specific options
    #[must_use]
    pub fn multi_select_options(
        mut self,
        multi_select_options: MultiSelectParameterOptions,
    ) -> Self {
        self.multi_select_options = Some(multi_select_options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `MultiSelectParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<MultiSelectParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description)
            .required(self.required)
            .build()?;

        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(MultiSelectParameter {
            metadata,
            default: self.default,
            options: self.options,
            multi_select_options: self.multi_select_options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// MultiSelectParameterOptions Builder
// =============================================================================

/// Builder for `MultiSelectParameterOptions`
#[derive(Debug, Default)]
pub struct MultiSelectParameterOptionsBuilder {
    min_selections: Option<usize>,
    max_selections: Option<usize>,
}

impl MultiSelectParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> MultiSelectParameterOptionsBuilder {
        MultiSelectParameterOptionsBuilder::default()
    }
}

impl MultiSelectParameterOptionsBuilder {
    /// Set minimum number of selections
    #[must_use]
    pub fn min_selections(mut self, min_selections: usize) -> Self {
        self.min_selections = Some(min_selections);
        self
    }

    /// Set maximum number of selections
    #[must_use]
    pub fn max_selections(mut self, max_selections: usize) -> Self {
        self.max_selections = Some(max_selections);
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> MultiSelectParameterOptions {
        MultiSelectParameterOptions {
            min_selections: self.min_selections,
            max_selections: self.max_selections,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for MultiSelectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::MultiSelect
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for MultiSelectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MultiSelectParameter({})", self.metadata.name)
    }
}

impl Validatable for MultiSelectParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Array)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_array().is_some_and(|arr| arr.is_empty())
    }
}

impl Displayable for MultiSelectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_select_parameter_builder() {
        let param = MultiSelectParameter::builder()
            .key("tags")
            .name("Tags")
            .description("Select one or more tags")
            .required(true)
            .options(vec![
                SelectOption::new("bug", "Bug", "bug"),
                SelectOption::new("feature", "Feature", "feature"),
            ])
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "tags");
        assert_eq!(param.metadata.name, "Tags");
        assert!(param.metadata.required);
        assert_eq!(param.options.len(), 2);
    }

    #[test]
    fn test_multi_select_parameter_with_default() {
        let param = MultiSelectParameter::builder()
            .key("categories")
            .name("Categories")
            .options(vec![
                SelectOption::new("a", "A", "a"),
                SelectOption::new("b", "B", "b"),
            ])
            .default(["a", "b"])
            .build()
            .unwrap();

        assert_eq!(param.default, Some(vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn test_multi_select_parameter_with_options() {
        let param = MultiSelectParameter::builder()
            .key("items")
            .name("Items")
            .options(vec![SelectOption::new("x", "X", "x")])
            .multi_select_options(
                MultiSelectParameterOptions::builder()
                    .min_selections(1)
                    .max_selections(5)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.multi_select_options.unwrap();
        assert_eq!(opts.min_selections, Some(1));
        assert_eq!(opts.max_selections, Some(5));
    }

    #[test]
    fn test_multi_select_parameter_missing_key() {
        let result = MultiSelectParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
