//! Resource parameter type for dynamic resource selection

use serde::Serialize;
use std::collections::HashMap;

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
    option::{OptionsResponse, Pagination, SelectOption},
};
use nebula_expression::MaybeExpression;
use nebula_value::{Value, ValueKind};

/// Context for resource loading
pub struct ResourceContext<'a> {
    /// Current parameter values
    pub parameters: &'a HashMap<String, MaybeExpression<Value>>,

    /// Search query (if any)
    pub search: Option<String>,

    /// Pagination parameters
    pub pagination: Option<Pagination>,

    /// Additional context data
    pub data: Option<nebula_value::Value>,
}

impl<'a> ResourceContext<'a> {
    /// Create a new resource context
    #[must_use]
    pub fn new(parameters: &'a HashMap<String, MaybeExpression<Value>>) -> Self {
        Self {
            parameters,
            search: None,
            pagination: None,
            data: None,
        }
    }

    /// Set search query
    #[must_use = "builder methods must be chained or built"]
    pub fn with_search(mut self, search: String) -> Self {
        self.search = Some(search);
        self
    }

    /// Set pagination
    #[must_use = "builder methods must be chained or built"]
    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }

    /// Set additional data
    #[must_use = "builder methods must be chained or built"]
    pub fn with_data(mut self, data: nebula_value::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Get a parameter value by key
    pub fn get(&self, key: &str) -> Option<&MaybeExpression<Value>> {
        self.parameters.get(key)
    }
}

/// Type alias for resource loader function
pub type ResourceLoader =
    Box<dyn Fn(&ResourceContext<'_>) -> Result<OptionsResponse, ParameterError> + Send + Sync>;

/// Resource value - simple string identifier
pub type ResourceValue = String;

/// Options for resource parameter
#[derive(Debug, Clone, Serialize)]
pub struct ResourceParameterOptions {
    /// Static fallback options (used if no loader)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub static_options: Option<Vec<SelectOption>>,

    /// Whether to support search
    #[serde(default)]
    pub searchable: bool,

    /// Whether to support pagination
    #[serde(default)]
    pub paginated: bool,

    /// Placeholder text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// Parameter for dynamic resource selection (like n8n's `ResourceLocator`)
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = ResourceParameter::builder()
///     .key("spreadsheet")
///     .name("Spreadsheet")
///     .description("Select a spreadsheet")
///     .options(ResourceParameterOptions {
///         static_options: None,
///         searchable: true,
///         paginated: true,
///         placeholder: Some("Select a spreadsheet...".into()),
///     })
///     .build()?;
/// ```
#[derive(Serialize)]
pub struct ResourceParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ResourceValue>,

    /// Resource loader for fetching options dynamically
    #[serde(skip)]
    pub loader: Option<ResourceLoader>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ResourceParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

impl std::fmt::Debug for ResourceParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceParameter")
            .field("metadata", &self.metadata)
            .field("default", &self.default)
            .field("loader", &self.loader.as_ref().map(|_| "<loader>"))
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

// =============================================================================
// ResourceParameter Builder
// =============================================================================

/// Builder for `ResourceParameter`
#[derive(Default)]
pub struct ResourceParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<ResourceValue>,
    loader: Option<ResourceLoader>,
    options: Option<ResourceParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl ResourceParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ResourceParameterBuilder {
        ResourceParameterBuilder::new()
    }
}

impl ResourceParameterBuilder {
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
            loader: None,
            options: None,
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

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Set the resource loader
    #[must_use]
    pub fn loader<F>(mut self, loader: F) -> Self
    where
        F: Fn(&ResourceContext<'_>) -> Result<OptionsResponse, ParameterError>
            + Send
            + Sync
            + 'static,
    {
        self.loader = Some(Box::new(loader));
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: ResourceParameterOptions) -> Self {
        self.options = Some(options);
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

    /// Build the `ResourceParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<ResourceParameter, ParameterError> {
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

        Ok(ResourceParameter {
            metadata,
            default: self.default,
            loader: self.loader,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for ResourceParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Resource
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for ResourceParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ResourceParameter({})", self.metadata.name)
    }
}

impl Validatable for ResourceParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value
            .as_text()
            .is_some_and(|s| s.as_str().trim().is_empty())
            || value.is_null()
    }
}

impl Displayable for ResourceParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl ResourceParameter {
    /// Load options using the loader
    pub fn load_options(
        &self,
        context: &ResourceContext<'_>,
    ) -> Result<OptionsResponse, ParameterError> {
        if let Some(loader) = &self.loader {
            loader(context)
        } else if let Some(opts) = &self.options {
            if let Some(static_opts) = &opts.static_options {
                Ok(OptionsResponse {
                    options: static_opts.clone(),
                    total: Some(static_opts.len()),
                    has_more: false,
                    next_cursor: None,
                })
            } else {
                Ok(OptionsResponse {
                    options: vec![],
                    total: Some(0),
                    has_more: false,
                    next_cursor: None,
                })
            }
        } else {
            Ok(OptionsResponse {
                options: vec![],
                total: Some(0),
                has_more: false,
                next_cursor: None,
            })
        }
    }

    /// Check if search is enabled
    #[must_use]
    pub fn is_searchable(&self) -> bool {
        self.options.as_ref().is_some_and(|opts| opts.searchable)
    }

    /// Check if pagination is enabled
    #[must_use]
    pub fn is_paginated(&self) -> bool {
        self.options.as_ref().is_some_and(|opts| opts.paginated)
    }

    /// Get placeholder text
    #[must_use]
    pub fn get_placeholder(&self) -> Option<&str> {
        self.options
            .as_ref()
            .and_then(|opts| opts.placeholder.as_deref())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_parameter_builder() {
        let param = ResourceParameter::builder()
            .key("spreadsheet")
            .name("Spreadsheet")
            .description("Select a spreadsheet")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "spreadsheet");
        assert_eq!(param.metadata.name, "Spreadsheet");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_resource_parameter_with_options() {
        let param = ResourceParameter::builder()
            .key("sheet")
            .name("Sheet")
            .options(ResourceParameterOptions {
                static_options: None,
                searchable: true,
                paginated: true,
                placeholder: Some("Select a sheet...".into()),
            })
            .build()
            .unwrap();

        assert!(param.is_searchable());
        assert!(param.is_paginated());
        assert_eq!(param.get_placeholder(), Some("Select a sheet..."));
    }

    #[test]
    fn test_resource_parameter_missing_key() {
        let result = ResourceParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }
}
