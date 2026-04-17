//! Runtime loader registry and async loader types.
//!
//! All fallible paths now return [`ValidationError`] (unified error type).
//! Codes emitted:
//!
//! | Code | When |
//! |------|------|
//! | `loader.not_registered` | Named loader key not found in registry |
//! | `loader.failed` | Loader invocation returned an error |
//!
//! Lint-time warnings (`missing_loader`, `loader_without_dynamic`) are emitted
//! by the lint pass in `lint.rs`, not here.

use std::{future::Future, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    FieldValues, SelectOption,
    error::ValidationError,
    key::FieldKey,
    path::{FieldPath, PathSegment},
};

/// Boxed future used by async loader functions.
pub type LoaderFuture<T> =
    Pin<Box<dyn Future<Output = Result<LoaderResult<T>, ValidationError>> + Send>>;

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
        Fut: Future<Output = Result<LoaderResult<T>, ValidationError>> + Send + 'static,
    {
        Self(Arc::new(move |context| Box::pin(loader(context))))
    }

    /// Execute loader for the provided context.
    pub async fn call(&self, context: LoaderContext) -> Result<LoaderResult<T>, ValidationError> {
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

/// Build a single-key `FieldPath` from a `LoaderContext::field_key` string.
/// Falls back to root if the key is not a valid `FieldKey`.
fn field_path_from_key(key: &str) -> FieldPath {
    FieldKey::new(key)
        .map(|fk| FieldPath::root().join(PathSegment::Key(fk)))
        .unwrap_or_else(|_| FieldPath::root())
}

/// Use the path from a loader-returned error if it's non-root, otherwise build
/// one from the registry lookup key.
fn field_path_from_err_or(key: &str, err: &ValidationError) -> FieldPath {
    if err.path.is_root() {
        field_path_from_key(key)
    } else {
        err.path.clone()
    }
}

/// Runtime registry for named loader functions.
#[derive(Debug, Clone, Default)]
pub struct LoaderRegistry {
    option_loaders: std::collections::HashMap<String, OptionLoader>,
    record_loaders: std::collections::HashMap<String, RecordLoader>,
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
        Fut: Future<Output = Result<LoaderResult<SelectOption>, ValidationError>> + Send + 'static,
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
        Fut: Future<Output = Result<LoaderResult<Value>, ValidationError>> + Send + 'static,
    {
        self.record_loaders
            .insert(key.into(), RecordLoader::new(loader));
        self
    }

    /// Resolve and execute option loader by key.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `loader.not_registered` when `key`
    /// is not registered, or `loader.failed` if the loader returns an error.
    /// Both errors carry the requesting field's path (from `context.field_key`).
    pub async fn load_options(
        &self,
        key: &str,
        context: LoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let field_path = field_path_from_key(&context.field_key);
        let Some(loader) = self.option_loaders.get(key) else {
            return Err(ValidationError::builder("loader.not_registered")
                .at(field_path)
                .message(format!("option loader `{key}` is not registered"))
                .param("loader", serde_json::Value::String(key.to_owned()))
                .build());
        };
        loader.call(context).await.map_err(|e| {
            ValidationError::builder("loader.failed")
                .at(field_path_from_err_or(key, &e))
                .message(format!("option loader `{key}` failed: {e}"))
                .param("loader", serde_json::Value::String(key.to_owned()))
                .source(e)
                .build()
        })
    }

    /// Resolve and execute record loader by key.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `loader.not_registered` when `key`
    /// is not registered, or `loader.failed` if the loader returns an error.
    /// Both errors carry the requesting field's path (from `context.field_key`).
    pub async fn load_records(
        &self,
        key: &str,
        context: LoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let field_path = field_path_from_key(&context.field_key);
        let Some(loader) = self.record_loaders.get(key) else {
            return Err(ValidationError::builder("loader.not_registered")
                .at(field_path)
                .message(format!("record loader `{key}` is not registered"))
                .param("loader", serde_json::Value::String(key.to_owned()))
                .build());
        };
        loader.call(context).await.map_err(|e| {
            ValidationError::builder("loader.failed")
                .at(field_path_from_err_or(key, &e))
                .message(format!("record loader `{key}` failed: {e}"))
                .param("loader", serde_json::Value::String(key.to_owned()))
                .source(e)
                .build()
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn load_options_unregistered_returns_not_registered() {
        let registry = LoaderRegistry::new();
        let ctx = LoaderContext::new("field", FieldValues::new());
        let err = registry.load_options("missing", ctx).await.unwrap_err();
        assert_eq!(err.code, "loader.not_registered");
        assert!(
            err.params
                .iter()
                .any(|(k, v)| k == "loader" && v == "missing")
        );
        // Error path should reflect the requesting field key.
        assert_eq!(err.path.to_string(), "field");
    }

    #[tokio::test]
    async fn load_records_unregistered_returns_not_registered() {
        let registry = LoaderRegistry::new();
        let ctx = LoaderContext::new("field", FieldValues::new());
        let err = registry.load_records("missing", ctx).await.unwrap_err();
        assert_eq!(err.code, "loader.not_registered");
        // Error path should reflect the requesting field key.
        assert_eq!(err.path.to_string(), "field");
    }

    #[tokio::test]
    async fn load_options_registered_returns_result() {
        let registry = LoaderRegistry::new().register_option("opts", |_ctx| async {
            Ok(LoaderResult::done(vec![SelectOption::new(
                json!("a"),
                "Option A",
            )]))
        });
        let ctx = LoaderContext::new("field", FieldValues::new());
        let result = registry.load_options("opts", ctx).await.unwrap();
        assert_eq!(result.items.len(), 1);
    }

    #[tokio::test]
    async fn loader_failure_wraps_as_loader_failed() {
        let registry = LoaderRegistry::new().register_option("fail", |_ctx| async {
            Err(ValidationError::builder("loader.failed")
                .message("downstream error")
                .build())
        });
        let ctx = LoaderContext::new("field", FieldValues::new());
        let err = registry.load_options("fail", ctx).await.unwrap_err();
        assert_eq!(err.code, "loader.failed");
    }

    #[test]
    fn loader_context_builder() {
        let ctx = LoaderContext::new("my_field", FieldValues::new())
            .with_filter("query")
            .with_cursor("tok")
            .with_metadata(json!({"page": 1}));
        assert_eq!(ctx.field_key, "my_field");
        assert_eq!(ctx.filter.as_deref(), Some("query"));
        assert_eq!(ctx.cursor.as_deref(), Some("tok"));
        assert!(ctx.metadata.is_some());
    }

    #[test]
    fn loader_result_constructors() {
        let r: LoaderResult<i32> = LoaderResult::done(vec![1, 2]);
        assert!(r.next_cursor.is_none());

        let p: LoaderResult<i32> = LoaderResult::page(vec![1], "next");
        assert_eq!(p.next_cursor.as_deref(), Some("next"));

        let t = p.with_total(100);
        assert_eq!(t.total, Some(100));
    }
}
