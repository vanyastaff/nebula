use serde::Serialize;
use std::collections::HashMap;

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
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
    #[must_use]
    pub fn new(parameters: &'a HashMap<String, MaybeExpression<Value>>) -> Self {
        Self {
            parameters,
            search: None,
            pagination: None,
            data: None,
        }
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn with_search(mut self, search: String) -> Self {
        self.search = Some(search);
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }

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
#[derive(Serialize)]
pub struct ResourceParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ResourceValue>,

    /// Resource loader for fetching options dynamically
    #[serde(skip)]
    pub loader: Option<ResourceLoader>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ResourceParameterOptions>,
}

impl std::fmt::Debug for ResourceParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceParameter")
            .field("base", &self.base)
            .field("default", &self.default)
            .field("loader", &self.loader.as_ref().map(|_| "<loader>"))
            .field("options", &self.options)
            .finish()
    }
}

impl ResourceParameter {
    /// Create a new resource parameter
    #[must_use]
    pub fn new(metadata: ParameterMetadata) -> Self {
        Self {
            base: ParameterBase::new(metadata),
            default: None,
            loader: None,
            options: None,
        }
    }

    /// Set options
    #[must_use = "builder methods must be chained or built"]
    pub fn with_options(mut self, options: ResourceParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set the resource loader
    #[must_use = "builder methods must be chained or built"]
    pub fn with_loader<F>(mut self, loader: F) -> Self
    where
        F: Fn(&ResourceContext<'_>) -> Result<OptionsResponse, ParameterError>
            + Send
            + Sync
            + 'static,
    {
        self.loader = Some(Box::new(loader));
        self
    }

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

impl Describable for ResourceParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Resource
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for ResourceParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ResourceParameter({})", self.base.metadata.name)
    }
}

impl Validatable for ResourceParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
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
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}
