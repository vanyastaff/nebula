//! Option types for select-based parameters
//!
//! This module provides types for handling static and dynamic options
//! for select, multi-select, and radio button parameters.

use std::borrow::Cow;
use std::fmt::Debug;
use bon::Builder;
use serde::{Deserialize, Serialize};

/// Context for loading dynamic options
/// 
/// This trait should be implemented by types that provide context
/// for dynamic option loading, such as database connections,
/// API clients, or other data sources.
pub trait OptionLoadContext: Send + Sync {}

// TODO: Add futures dependency for dynamic option loading
// /// Function signature for loading dynamic options
// ///
// /// This function takes a context, optional search query, and pagination
// /// parameters, and returns a future that resolves to an options response.
// pub type OptionLoader = Arc<
//     dyn Fn(
//         Box<dyn OptionLoadContext>,
//         Option<String>,    // search query
//         Option<Pagination> // pagination
//     ) -> futures::future::BoxFuture<'static, Result<OptionsResponse, ParameterError>>
//     + Send
//     + Sync
// >;

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

/// Options configuration for select-based parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SelectOptions {
    /// Static options defined at compile time
    Static(Vec<SelectOption>),

    // TODO: Add back when futures dependency is available
    // /// Dynamic options fetched from a function
    // Dynamic(DynamicOptions),
}

impl SelectOptions {
    /// Create static options
    pub fn static_options(options: Vec<SelectOption>) -> Self {
        SelectOptions::Static(options)
    }

    // TODO: Add back when futures dependency is available
    // /// Create dynamic options
    // pub fn dynamic_options(name: String, loader: OptionLoader) -> Self {
    //     SelectOptions::Dynamic(DynamicOptions { name, loader })
    // }
}

// TODO: Add back when futures dependency is available
// /// Configuration for dynamically loaded options
// #[derive(Clone, Serialize)]
// pub struct DynamicOptions {
//     /// Identifier for the dynamic options loader
//     pub name: String,
//
//     /// Function to load options dynamically
//     #[serde(skip)]
//     pub loader: OptionLoader,
// }
//
// impl<'de> serde::Deserialize<'de> for DynamicOptions {
//     fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
//     where
//         D: serde::Deserializer<'de>,
//     {
//         #[derive(Deserialize)]
//         struct DynamicOptionsHelper {
//             name: String,
//         }
//
//         let helper = DynamicOptionsHelper::deserialize(deserializer)?;
//
//         // Create a dummy loader that returns an error
//         let dummy_loader: OptionLoader = Arc::new(|_, _, _| {
//             Box::pin(async {
//                 Err(ParameterError::InvalidValue {
//                     param: "dynamic_options".to_string(),
//                     reason: "Loader function not available after deserialization".to_string(),
//                 })
//             })
//         });
//
//         Ok(DynamicOptions {
//             name: helper.name,
//             loader: dummy_loader,
//         })
//     }
// }
//
// impl Debug for DynamicOptions {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("DynamicOptions")
//             .field("name", &self.name)
//             .field("loader", &"<OptionLoader>")
//             .finish()
//     }
// }

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Builder)]
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
    /// Create a simple option with key, name, and value
    pub fn new(key: impl Into<Cow<'static, str>>, name: impl Into<String>, value: impl Into<String>) -> Self {
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

    /// Create a simple option where key and value are the same
    pub fn simple(key_value: impl Into<String>, name: impl Into<String>) -> Self {
        let key_value = key_value.into();
        Self::new(key_value.clone(), name, key_value)
    }

    /// Create an option with a description
    pub fn with_description(
        key: impl Into<Cow<'static, str>>, 
        name: impl Into<String>, 
        value: impl Into<String>,
        description: impl Into<Cow<'static, str>>
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            value: value.into(),
            description: Some(description.into()),
            icon: None,
            disabled: None,
            group: None,
            color: None,
            subtitle: None,
        }
    }
}
