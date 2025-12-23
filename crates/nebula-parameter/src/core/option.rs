//! Option types for select-based parameters
//!
//! This module provides types for handling static and dynamic options
//! for select, multi-select, and radio button parameters.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Debug;

use crate::core::ParameterError;
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Pagination parameters for loading options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    /// Page number (0-based)
    pub page: usize,

    /// Items per page
    pub page_size: usize,

    /// Optional cursor for cursor-based pagination
    pub cursor: Option<String>,
}

/// Response containing a page of options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionsResponse {
    /// The options for this page
    pub options: Vec<SelectOption>,

    /// Total number of options available
    pub total: Option<usize>,

    /// Whether there are more pages
    pub has_more: bool,

    /// Cursor for next page (if using cursor pagination)
    pub next_cursor: Option<String>,
}

/// Context for loading dynamic options
#[derive(Debug)]
pub struct OptionLoadContext<'a> {
    /// Current parameter values for dependency resolution
    pub parameters: &'a HashMap<String, MaybeExpression<Value>>,

    /// Optional search query
    pub search: Option<String>,

    /// Optional pagination parameters
    pub pagination: Option<Pagination>,
}

impl<'a> OptionLoadContext<'a> {
    #[must_use]
    pub fn new(parameters: &'a HashMap<String, MaybeExpression<Value>>) -> Self {
        Self {
            parameters,
            search: None,
            pagination: None,
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
}

/// Type alias for option loader function
pub type OptionLoader =
    Box<dyn Fn(&OptionLoadContext<'_>) -> Result<OptionsResponse, ParameterError> + Send + Sync>;

/// Options configuration for select-based parameters
pub enum SelectOptions {
    /// Static options defined at compile time
    Static(Vec<SelectOption>),
    /// Dynamic options loaded from a function
    Dynamic(DynamicOptions),
}

impl SelectOptions {
    /// Create static options
    #[must_use]
    pub fn static_options(options: Vec<SelectOption>) -> Self {
        SelectOptions::Static(options)
    }

    /// Create dynamic options
    #[must_use]
    pub fn dynamic_options(loader: OptionLoader) -> Self {
        SelectOptions::Dynamic(DynamicOptions { loader })
    }
}

impl Clone for SelectOptions {
    fn clone(&self) -> Self {
        match self {
            Self::Static(options) => Self::Static(options.clone()),
            Self::Dynamic(_) => {
                // Cannot clone dynamic options with closures
                // Return empty static as fallback
                Self::Static(Vec::new())
            }
        }
    }
}

impl Debug for SelectOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Static(options) => f.debug_tuple("Static").field(options).finish(),
            Self::Dynamic(_) => f.debug_tuple("Dynamic").field(&"<loader>").finish(),
        }
    }
}

impl Serialize for SelectOptions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Static(options) => {
                #[derive(Serialize)]
                struct StaticOptions<'a> {
                    r#type: &'static str,
                    options: &'a Vec<SelectOption>,
                }
                StaticOptions {
                    r#type: "static",
                    options,
                }
                .serialize(serializer)
            }
            Self::Dynamic(_) => {
                #[derive(Serialize)]
                struct DynamicOptions {
                    r#type: &'static str,
                }
                DynamicOptions { r#type: "dynamic" }.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for SelectOptions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            r#type: String,
            #[serde(default)]
            options: Vec<SelectOption>,
        }

        let helper = Helper::deserialize(deserializer)?;
        match helper.r#type.as_str() {
            "static" => Ok(Self::Static(helper.options)),
            "dynamic" => {
                // Create a dummy loader that returns empty options
                let loader: OptionLoader = Box::new(|_| {
                    Ok(OptionsResponse {
                        options: vec![],
                        total: Some(0),
                        has_more: false,
                        next_cursor: None,
                    })
                });
                Ok(Self::Dynamic(DynamicOptions { loader }))
            }
            _ => Err(serde::de::Error::custom(format!(
                "Unknown options type: {}",
                helper.r#type
            ))),
        }
    }
}

/// Configuration for dynamically loaded options
pub struct DynamicOptions {
    /// Function to load options dynamically
    pub loader: OptionLoader,
}

impl DynamicOptions {
    /// Load options using the loader
    #[must_use = "loaded options must be used"]
    pub fn load(&self, context: &OptionLoadContext<'_>) -> Result<OptionsResponse, ParameterError> {
        (self.loader)(context)
    }
}

/// A single option for select-based parameters
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::SelectOption;
///
/// // Simple creation
/// let option = SelectOption::new("key1", "Display Name", "value1");
///
/// // Using builder with Into conversions
/// let option = SelectOption::builder()
///     .key("api_key")
///     .name("API Key Authentication")
///     .value("api_key")
///     .description("Use API key for authentication")
///     .icon("key")
///     .build();
/// ```
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, bon::Builder)]
#[builder(on(String, into), on(Cow<'_, str>, into))]
pub struct SelectOption {
    /// Unique key for the option
    pub key: Cow<'static, str>,

    /// Display name
    pub name: String,

    /// Option value
    pub value: String,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Cow<'static, str>>,

    /// Optional icon (icon name)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<Cow<'static, str>>,

    /// Whether this option is disabled
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,

    /// Group name for grouping options
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Cow<'static, str>>,

    /// Color hint (hex or named color)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Cow<'static, str>>,

    /// Additional subtitle text
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

impl SelectOption {
    /// Creates a new option with required fields.
    ///
    /// This is a convenience constructor for simple cases. For advanced
    /// configuration (description, icon, color, etc.), use [`SelectOption::builder()`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use nebula_parameter::SelectOption;
    ///
    /// // Simple option
    /// let option = SelectOption::new("key", "Display Name", "value");
    ///
    /// // For additional fields, use the builder:
    /// let option = SelectOption::builder()
    ///     .key("key")
    ///     .name("Name")
    ///     .value("value")
    ///     .description("Description")
    ///     .icon("icon-name")
    ///     .build();
    /// ```
    #[inline]
    pub fn new(
        key: impl Into<Cow<'static, str>>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            value: value.into(),
            description: None,
            icon: None,
            disabled: None,
            group: None,
            color: None,
            subtitle: None,
        }
    }
}
