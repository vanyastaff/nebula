use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{FieldValues, SelectOption};

/// Boxed future used by async loader functions.
pub type LoaderFuture<T> =
    Pin<Box<dyn Future<Output = Result<LoaderResult<T>, LoaderError>> + Send>>;

/// Error returned by runtime loaders.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LoaderError {
    /// Human-readable description of the loader failure.
    pub message: String,
    /// Optional source error for chaining.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl LoaderError {
    /// Build a loader error with message only.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Build a loader error and preserve source details.
    pub fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// Context passed to runtime loaders.
#[derive(Debug, Clone)]
pub struct LoaderContext {
    /// Key of the field currently requesting dynamic data.
    pub field_key: String,
    /// Current runtime values at call time.
    pub values: FieldValues,
    /// Optional free-text query from searchable UI controls.
    pub filter: Option<String>,
    /// Optional pagination cursor from previous response.
    pub cursor: Option<String>,
    /// Loader-specific metadata.
    pub metadata: Option<Value>,
}

impl LoaderContext {
    /// Construct context for a specific field key.
    pub fn new(field_key: impl Into<String>, values: FieldValues) -> Self {
        Self {
            field_key: field_key.into(),
            values,
            filter: None,
            cursor: None,
            metadata: None,
        }
    }

    /// Attach text filter.
    #[must_use]
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Attach pagination cursor.
    #[must_use]
    pub fn with_cursor(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    /// Attach arbitrary metadata payload.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Generic async loader wrapper.
pub struct Loader<T: Send + 'static>(
    Arc<dyn Fn(LoaderContext) -> LoaderFuture<T> + Send + Sync + 'static>,
);

impl<T: Send + 'static> Loader<T> {
    /// Wrap an async closure into a reusable loader object.
    pub fn new<F, Fut>(loader: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<T>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |context| Box::pin(loader(context))))
    }

    /// Execute loader for the provided context.
    pub async fn call(&self, context: LoaderContext) -> Result<LoaderResult<T>, LoaderError> {
        (self.0)(context).await
    }
}

impl<T: Send + 'static> Clone for Loader<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Send + 'static> PartialEq for Loader<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T: Send + 'static> std::fmt::Debug for Loader<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Loader(<async fn>)")
    }
}

/// Paginated result returned from runtime loaders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoaderResult<T> {
    /// Page of resolved items.
    pub items: Vec<T>,
    /// Cursor for fetching next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Optional total item count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl<T> LoaderResult<T> {
    /// Build a non-paginated result.
    pub fn done(items: Vec<T>) -> Self {
        Self {
            items,
            next_cursor: None,
            total: None,
        }
    }

    /// Build a paginated result with next cursor.
    pub fn page(items: Vec<T>, cursor: impl Into<String>) -> Self {
        Self {
            items,
            next_cursor: Some(cursor.into()),
            total: None,
        }
    }

    /// Attach total count.
    #[must_use]
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }
}

impl<T> From<Vec<T>> for LoaderResult<T> {
    fn from(items: Vec<T>) -> Self {
        Self::done(items)
    }
}

/// Loader returning select options.
pub type OptionLoader = Loader<SelectOption>;
/// Loader returning dynamic record payloads.
pub type RecordLoader = Loader<Value>;

/// Runtime registry for named loader functions.
#[derive(Debug, Clone, Default)]
pub struct LoaderRegistry {
    option_loaders: HashMap<String, OptionLoader>,
    record_loaders: HashMap<String, RecordLoader>,
}

impl LoaderRegistry {
    /// Create an empty loader registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register option loader using builder style.
    #[must_use]
    pub fn register_option<F, Fut>(mut self, key: impl Into<String>, loader: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<SelectOption>, LoaderError>> + Send + 'static,
    {
        self.option_loaders
            .insert(key.into(), OptionLoader::new(loader));
        self
    }

    /// Register record loader using builder style.
    #[must_use]
    pub fn register_record<F, Fut>(mut self, key: impl Into<String>, loader: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<Value>, LoaderError>> + Send + 'static,
    {
        self.record_loaders
            .insert(key.into(), RecordLoader::new(loader));
        self
    }

    /// Resolve and execute option loader by key.
    pub async fn load_options(
        &self,
        key: &str,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, LoaderError> {
        let Some(loader) = self.option_loaders.get(key) else {
            return Err(LoaderError::new(format!(
                "option loader `{key}` is not registered"
            )));
        };
        loader.call(context).await
    }

    /// Resolve and execute record loader by key.
    pub async fn load_records(
        &self,
        key: &str,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, LoaderError> {
        let Some(loader) = self.record_loaders.get(key) else {
            return Err(LoaderError::new(format!(
                "record loader `{key}` is not registered"
            )));
        };
        loader.call(context).await
    }
}
