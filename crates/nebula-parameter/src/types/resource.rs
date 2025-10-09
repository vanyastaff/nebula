use serde::Serialize;
use std::collections::HashMap;

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
    option::{OptionsResponse, Pagination, SelectOption},
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

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
    pub fn new(parameters: &'a HashMap<String, MaybeExpression<Value>>) -> Self {
        Self {
            parameters,
            search: None,
            pagination: None,
            data: None,
        }
    }

    pub fn with_search(mut self, search: String) -> Self {
        self.search = Some(search);
        self
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }

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

/// Parameter for dynamic resource selection (like n8n's ResourceLocator)
#[derive(Serialize)]
pub struct ResourceParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ResourceValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ResourceValue>,

    /// Resource loader for fetching options dynamically
    #[serde(skip)]
    pub loader: Option<ResourceLoader>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ResourceParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

impl std::fmt::Debug for ResourceParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceParameter")
            .field("metadata", &self.metadata)
            .field("value", &self.value)
            .field("default", &self.default)
            .field("loader", &self.loader.as_ref().map(|_| "<loader>"))
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

impl ResourceParameter {
    /// Create a new resource parameter
    pub fn new(metadata: ParameterMetadata) -> Self {
        Self {
            metadata,
            value: None,
            default: None,
            loader: None,
            options: None,
            display: None,
            validation: None,
        }
    }

    /// Set options
    pub fn with_options(mut self, options: ResourceParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set the resource loader
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
    pub fn is_searchable(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.searchable)
            .unwrap_or(false)
    }

    /// Check if pagination is enabled
    pub fn is_paginated(&self) -> bool {
        self.options
            .as_ref()
            .map(|opts| opts.paginated)
            .unwrap_or(false)
    }

    /// Get placeholder text
    pub fn get_placeholder(&self) -> Option<&str> {
        self.options
            .as_ref()
            .and_then(|opts| opts.placeholder.as_deref())
    }
}

impl Parameter for ResourceParameter {
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

impl HasValue for ResourceParameter {
    type Value = ResourceValue;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

#[async_trait::async_trait]
impl Expressible for ResourceParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value.as_ref().map(|v| {
            MaybeExpression::Value(nebula_value::Value::Text(nebula_value::Text::from(
                v.clone(),
            )))
        })
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            MaybeExpression::Value(nebula_value::Value::Text(s)) => {
                self.value = Some(s.to_string());
                Ok(())
            }
            MaybeExpression::Expression(expr) => {
                self.value = Some(expr);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for resource".to_string(),
            }),
        }
    }
}

impl Validatable for ResourceParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.is_empty()
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
