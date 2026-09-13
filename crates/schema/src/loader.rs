//! Runtime loader registry and async loader types.
//!
//! All fallible paths now return [`ValidationError`] (unified error type).
//! Codes emitted:
//!
//! | Code | When |
//! |------|------|
//! | `loader.not_registered` | Named loader key not found in registry |
//! | `loader.failed` | Loader invocation returned an error |
//! | `loader.result_too_large` | A loader page exceeded an item resource limit |
//! | `recursion_limit` | A context exceeded the value depth limit before dispatch |
//!
//! Lint-time warnings (`missing_loader`, `loader_without_dynamic`) are emitted
//! by the lint pass in `lint.rs`, not here.
//!
//! # Resource bounds (what this layer does and does NOT enforce)
//!
//! The registry bounds page length, cumulative serialized item bytes, returned
//! item depth, and value snapshot depth before dispatch. It does
//! **not** apply a **timeout**, **rate limit**, or **cache**: those need a
//! runtime, a clock, and a tenant identity that this crate deliberately has none
//! of (mirroring its `validator` / `expression` peers). The caller wiring a
//! loader (e.g. the engine) MUST wrap each call in its runtime's timeout (a hung
//! loader otherwise blocks validation indefinitely) and rate-limit per tenant /
//! loader key. If it caches, the cache key MUST capture **everything the loader's
//! output depends on** — the loader key, the **tenant**, and the `LoaderContext`
//! inputs the loader reads (`values`, `filter`, `cursor`, `metadata`). A loader
//! whose result depends on runtime `values` (e.g. a team-scoped option list) must
//! never share a cache entry across tenants or differing contexts; a partial key
//! such as `(loader_key, filter, cursor)` alone would leak one context's page to
//! another.

use std::{future::Future, io::Write, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AuthoredValue, SelectOption, ValuePath, error::ValidationError, field::Field,
    path::FieldPath as SchemaPath,
};

/// Boxed future used by async loader functions.
pub type LoaderFuture<T> =
    Pin<Box<dyn Future<Output = Result<LoaderResult<T>, ValidationError>> + Send>>;

/// Request context before or after schema-bound redaction.
///
/// A raw context may contain unvalidated authored input. Schema entrypoints
/// apply `redacted` before dispatch; loaders receive data-only snapshots.
#[derive(Clone)]
pub struct LoaderContext {
    /// Key or schema path of the field currently requesting dynamic data.
    pub field_key: String,
    /// Input values, or the data-only snapshot after schema-bound redaction.
    pub values: AuthoredValue,
    /// Optional free-text query from searchable UI controls.
    pub filter: Option<String>,
    /// Optional pagination cursor from previous response.
    pub cursor: Option<String>,
    /// Loader-specific metadata.
    pub metadata: Option<Value>,
}

impl std::fmt::Debug for LoaderContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoaderContext")
            .finish_non_exhaustive()
    }
}

impl LoaderContext {
    /// Construct a raw request for a specific field key.
    pub fn new(field_key: impl Into<String>, values: AuthoredValue) -> Self {
        Self {
            field_key: field_key.into(),
            values,
            filter: None,
            cursor: None,
            metadata: None,
        }
    }

    /// Produces an inspectable schema-bound snapshot without making it
    /// executable. Loader dispatch remains available only through
    /// [`Schema`](struct@crate::Schema) and [`ValidSchema`](struct@crate::ValidSchema)
    /// entrypoints.
    ///
    /// # Errors
    ///
    /// Returns `recursion_limit` before copying input deeper than the value limit.
    pub fn with_secrets_redacted(
        self,
        schema: &crate::validated::ValidSchema,
    ) -> Result<RedactedLoaderContext, ValidationError> {
        self.redacted(schema.fields())
    }

    /// Schema and ValidSchema both bind loader input through this boundary.
    #[tracing::instrument(level = "debug", skip_all, fields(field_count = fields.len()))]
    pub(crate) fn redacted(
        mut self,
        fields: &[Field],
    ) -> Result<RedactedLoaderContext, ValidationError> {
        let snapshot = crate::context::redacted_loader_json(fields, &self.values)?;
        self.values = AuthoredValue::from_data(snapshot)?;
        Ok(RedactedLoaderContext(self))
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

/// Proof that a loader request was scrubbed against its owning schema.
///
/// Construction is crate-private so executable loader paths cannot accept an
/// unbound [`LoaderContext`].
pub struct RedactedLoaderContext(LoaderContext);

impl RedactedLoaderContext {
    /// Returns the data-only, schema-scrubbed value snapshot.
    #[must_use]
    pub fn values(&self) -> &AuthoredValue {
        &self.0.values
    }
}

impl std::fmt::Debug for RedactedLoaderContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedactedLoaderContext")
            .finish_non_exhaustive()
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

    /// Executes a schema-scrubbed request.
    pub(crate) async fn call(
        &self,
        context: RedactedLoaderContext,
    ) -> Result<LoaderResult<T>, ValidationError> {
        (self.0)(context.0).await
    }
}

impl<T: Send + 'static> Clone for Loader<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

// Loader closures have no meaningful value equality.

impl<T: Send + 'static> std::fmt::Debug for Loader<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Loader(<async fn>)")
    }
}

/// Paginated result returned from runtime loaders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[must_use]
    pub const fn done(items: Vec<T>) -> Self {
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
    pub const fn with_total(mut self, total: u64) -> Self {
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

/// Build a runtime data path from a loader request's pointer or schema path.
///
/// Falls back to root if the string is not a valid schema path.
fn field_path_from_key(key: &str) -> ValuePath {
    if key.starts_with('/') {
        ValuePath::parse(key).unwrap_or_else(ValuePath::root)
    } else {
        SchemaPath::parse(key).map_or_else(|_| ValuePath::root(), Into::into)
    }
}

/// Use the path from a loader-returned error if it's non-root, otherwise fall
/// back to the request field path.
fn field_path_from_err_or(fallback: &ValuePath, err: &ValidationError) -> ValuePath {
    if err.path().is_root() {
        fallback.clone()
    } else {
        err.path().clone()
    }
}

/// Hard ceiling on the number of items a single loader page may return.
///
/// A loader returning more than this fails `loader.result_too_large` instead of
/// flowing an unbounded result into validation, the UI, or serialization — a
/// misbehaving (or hostile) loader must paginate via [`LoaderResult::next_cursor`].
/// Item bytes and depth have separate ceilings below. Timeout, rate-limit, and
/// cache concerns belong to the caller's runtime (see [`LoaderRegistry`]).
pub const MAX_LOADER_ITEMS: usize = 10_000;

/// Hard ceiling on cumulative serialized item bytes in one loader page.
pub const MAX_LOADER_PAGE_BYTES: usize = 1_048_576;

/// Hard ceiling on JSON nesting inside one loader item.
pub const MAX_LOADER_ITEM_DEPTH: u8 = crate::value::MAX_VALUE_DEPTH;

trait LoaderPageItem: Serialize {
    fn nested_value(&self) -> &Value;
}

impl LoaderPageItem for Value {
    fn nested_value(&self) -> &Value {
        self
    }
}

impl LoaderPageItem for SelectOption {
    fn nested_value(&self) -> &Value {
        &self.value
    }
}

fn enforce_page_bounds<T: LoaderPageItem>(
    result: LoaderResult<T>,
    key: &str,
    path: &ValuePath,
) -> Result<LoaderResult<T>, ValidationError> {
    let count = result.items.len();
    if count > MAX_LOADER_ITEMS {
        return Err(loader_page_limit_error(
            key,
            path,
            "item count",
            count,
            MAX_LOADER_ITEMS,
        ));
    }

    let mut byte_counter = LoaderPageByteCounter::new(MAX_LOADER_PAGE_BYTES);
    for item in &result.items {
        let item_depth = json_depth(item.nested_value());
        if item_depth > usize::from(MAX_LOADER_ITEM_DEPTH) {
            return Err(loader_page_limit_error(
                key,
                path,
                "item depth",
                item_depth,
                usize::from(MAX_LOADER_ITEM_DEPTH),
            ));
        }
        if let Err(error) = serde_json::to_writer(&mut byte_counter, item) {
            if byte_counter.exceeded {
                return Err(loader_page_limit_error(
                    key,
                    path,
                    "serialized item bytes",
                    byte_counter.attempted_bytes,
                    MAX_LOADER_PAGE_BYTES,
                ));
            }
            return Err(ValidationError::builder("loader.failed")
                .at(path.clone())
                .message("loader result encoding failed")
                .param("loader", Value::String(key.to_owned()))
                .private_source(error)
                .build());
        }
    }
    Ok(result)
}

fn json_depth(value: &Value) -> usize {
    let mut maximum_depth = 0;
    let mut pending = vec![(value, 0_usize)];
    while let Some((value, depth)) = pending.pop() {
        maximum_depth = maximum_depth.max(depth);
        match value {
            Value::Array(values) => {
                pending.extend(values.iter().map(|value| (value, depth.saturating_add(1))));
            },
            Value::Object(values) => {
                pending.extend(
                    values
                        .values()
                        .map(|value| (value, depth.saturating_add(1))),
                );
            },
            _ => {},
        }
    }
    maximum_depth
}

fn loader_page_limit_error(
    key: &str,
    path: &ValuePath,
    resource: &'static str,
    count: usize,
    limit: usize,
) -> ValidationError {
    tracing::warn!(
        target: "nebula_schema::loader",
        loader_key = %key,
        resource,
        count,
        limit,
        "loader page resource limit exceeded"
    );
    ValidationError::builder("loader.result_too_large")
        .at(path.clone())
        .message("loader page exceeds a resource limit")
        .param("loader", Value::String(key.to_owned()))
        .param("resource", resource)
        .param("count", usize_as_u64(count))
        .param("limit", usize_as_u64(limit))
        .build()
}

fn usize_as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

struct LoaderPageByteCounter {
    written_bytes: usize,
    attempted_bytes: usize,
    limit_bytes: usize,
    exceeded: bool,
}

impl LoaderPageByteCounter {
    const fn new(limit_bytes: usize) -> Self {
        Self {
            written_bytes: 0,
            attempted_bytes: 0,
            limit_bytes,
            exceeded: false,
        }
    }
}

impl Write for LoaderPageByteCounter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.attempted_bytes = self.written_bytes.saturating_add(buffer.len());
        if self.attempted_bytes > self.limit_bytes {
            self.exceeded = true;
            return Err(std::io::Error::other("loader page byte limit exceeded"));
        }
        self.written_bytes = self.attempted_bytes;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
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
    #[must_use]
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
    ///
    /// cancel-safe: the registry holds no mutable state; a cancelled call drops
    /// the in-flight loader future (see [`Loader::call`]).
    #[tracing::instrument(
        level = "info",
        target = "nebula_schema::loader",
        skip(self, context),
        fields(loader_key = %key, field_key = %context.0.field_key)
    )]
    pub(crate) async fn load_options(
        &self,
        key: &str,
        context: RedactedLoaderContext,
    ) -> Result<LoaderResult<SelectOption>, ValidationError> {
        let field_path = field_path_from_key(&context.0.field_key);
        let Some(loader) = self.option_loaders.get(key) else {
            tracing::warn!(
                target: "nebula_schema::loader",
                loader_key = %key,
                "option loader not registered"
            );
            return Err(ValidationError::builder("loader.not_registered")
                .at(field_path)
                .message(format!("option loader `{key}` is not registered"))
                .param("loader", Value::String(key.to_owned()))
                .build());
        };
        let result = loader.call(context).await.map_err(|e| {
            tracing::warn!(
                target: "nebula_schema::loader",
                loader_key = %key,
                code = %e.code(),
                "option loader call failed"
            );
            ValidationError::builder("loader.failed")
                .at(field_path_from_err_or(&field_path, &e))
                .message("option loader failed")
                .param("loader", Value::String(key.to_owned()))
                .private_source(e)
                .build()
        })?;
        enforce_page_bounds(result, key, &field_path)
    }

    /// Resolve and execute record loader by key.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `loader.not_registered` when `key`
    /// is not registered, or `loader.failed` if the loader returns an error.
    /// Both errors carry the requesting field's path (from `context.field_key`).
    ///
    /// cancel-safe: the registry holds no mutable state; a cancelled call drops
    /// the in-flight loader future (see [`Loader::call`]).
    #[tracing::instrument(
        level = "info",
        target = "nebula_schema::loader",
        skip(self, context),
        fields(loader_key = %key, field_key = %context.0.field_key)
    )]
    pub(crate) async fn load_records(
        &self,
        key: &str,
        context: RedactedLoaderContext,
    ) -> Result<LoaderResult<Value>, ValidationError> {
        let field_path = field_path_from_key(&context.0.field_key);
        let Some(loader) = self.record_loaders.get(key) else {
            tracing::warn!(
                target: "nebula_schema::loader",
                loader_key = %key,
                "record loader not registered"
            );
            return Err(ValidationError::builder("loader.not_registered")
                .at(field_path)
                .message(format!("record loader `{key}` is not registered"))
                .param("loader", Value::String(key.to_owned()))
                .build());
        };
        let result = loader.call(context).await.map_err(|e| {
            tracing::warn!(
                target: "nebula_schema::loader",
                loader_key = %key,
                code = %e.code(),
                "record loader call failed"
            );
            ValidationError::builder("loader.failed")
                .at(field_path_from_err_or(&field_path, &e))
                .message("record loader failed")
                .param("loader", Value::String(key.to_owned()))
                .private_source(e)
                .build()
        })?;
        enforce_page_bounds(result, key, &field_path)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        AuthoredValue, Field, ScalarValue, Schema, ValueTree, expression::Expression, field_key,
        secret::SECRET_REDACTED,
    };

    fn scrub(context: LoaderContext) -> RedactedLoaderContext {
        context.redacted(&[]).unwrap()
    }

    #[tokio::test]
    async fn load_options_unregistered_returns_not_registered() {
        let registry = LoaderRegistry::new();
        let ctx = LoaderContext::new("field", AuthoredValue::object());
        let err = registry
            .load_options("missing", scrub(ctx))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "loader.not_registered");
        assert!(
            err.params()
                .iter()
                .any(|(k, v)| k == "loader" && v == "missing")
        );
        // Error path should reflect the requesting field key.
        assert_eq!(err.path().to_string(), "/field");
    }

    #[tokio::test]
    async fn load_records_unregistered_returns_not_registered() {
        let registry = LoaderRegistry::new();
        let ctx = LoaderContext::new("field", AuthoredValue::object());
        let err = registry
            .load_records("missing", scrub(ctx))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "loader.not_registered");
        // Error path should reflect the requesting field key.
        assert_eq!(err.path().to_string(), "/field");
    }

    #[tokio::test]
    async fn load_options_registered_returns_result() {
        let registry = LoaderRegistry::new().register_option("opts", |_ctx| async {
            Ok(LoaderResult::done(vec![SelectOption::new(
                json!("a"),
                "Option A",
            )]))
        });
        let ctx = LoaderContext::new("field", AuthoredValue::object());
        let result = registry.load_options("opts", scrub(ctx)).await.unwrap();
        assert_eq!(result.items.len(), 1);
    }

    #[tokio::test]
    async fn loader_failure_wraps_as_loader_failed() {
        const PRIVATE_DETAIL: &str = "PRIVATE downstream loader detail";
        let registry = LoaderRegistry::new().register_option("fail", |_ctx| async {
            Err(ValidationError::builder("loader.failed")
                .message(PRIVATE_DETAIL)
                .build())
        });
        let ctx = LoaderContext::new("field", AuthoredValue::object());
        let err = registry.load_options("fail", scrub(ctx)).await.unwrap_err();
        assert_eq!(err.code(), "loader.failed");
        assert_eq!(err.message(), "option loader failed");
        assert!(!format!("{err:?} {err}").contains(PRIVATE_DETAIL));
        assert_eq!(
            std::error::Error::source(&err).map(ToString::to_string),
            Some("diagnostic details contain private input".to_owned())
        );
    }

    #[tokio::test]
    async fn record_loader_failure_keeps_source_private() {
        const PRIVATE_DETAIL: &str = "PRIVATE record loader detail";
        let registry = LoaderRegistry::new().register_record("fail", |_ctx| async {
            Err(ValidationError::builder("loader.failed")
                .message(PRIVATE_DETAIL)
                .build())
        });
        let context = LoaderContext::new("field", AuthoredValue::object());
        let error = registry
            .load_records("fail", scrub(context))
            .await
            .unwrap_err();
        assert_eq!(error.message(), "record loader failed");
        assert!(!format!("{error:?} {error}").contains(PRIVATE_DETAIL));
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some("diagnostic details contain private input".to_owned())
        );
    }

    #[tokio::test]
    async fn loader_failure_root_path_maps_back_to_request_field() {
        let registry = LoaderRegistry::new().register_option("regions_loader", |_ctx| async {
            Err(ValidationError::builder("loader.failed")
                .at(ValuePath::root())
                .message("downstream error")
                .build())
        });
        let ctx = LoaderContext::new("region", AuthoredValue::object());
        let err = registry
            .load_options("regions_loader", scrub(ctx))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "loader.failed");
        assert_eq!(err.path().to_string(), "/region");
    }

    #[tokio::test]
    async fn load_options_rejects_oversized_result() {
        // A loader page above the item ceiling fails closed (it must paginate).
        let registry = LoaderRegistry::new().register_option("big", |_ctx| async {
            let items = (0..=MAX_LOADER_ITEMS)
                .map(|i| SelectOption::new(json!(i), format!("o{i}")))
                .collect();
            Ok(LoaderResult::done(items))
        });
        let ctx = LoaderContext::new("region", AuthoredValue::object());
        let err = registry.load_options("big", scrub(ctx)).await.unwrap_err();
        assert_eq!(err.code(), "loader.result_too_large");
        assert_eq!(
            err.path().to_string(),
            "/region",
            "carries the requesting field"
        );
        assert!(
            err.params()
                .iter()
                .any(|(k, v)| k == "limit" && v.as_u64() == Some(MAX_LOADER_ITEMS as u64)),
            "reports the limit: {:?}",
            err.params()
        );
    }

    #[tokio::test]
    async fn load_options_accepts_max_items_boundary() {
        // Exactly the ceiling is allowed; only strictly above it is rejected.
        let registry = LoaderRegistry::new().register_option("atlimit", |_ctx| async {
            let items = (0..MAX_LOADER_ITEMS)
                .map(|i| SelectOption::new(json!(i), format!("o{i}")))
                .collect();
            Ok(LoaderResult::done(items))
        });
        let ctx = LoaderContext::new("region", AuthoredValue::object());
        let result = registry.load_options("atlimit", scrub(ctx)).await.unwrap();
        assert_eq!(result.items.len(), MAX_LOADER_ITEMS);
    }

    #[tokio::test]
    async fn load_records_rejects_oversized_result() {
        let registry = LoaderRegistry::new().register_record("big", |_ctx| async {
            let items = (0..=MAX_LOADER_ITEMS).map(|i| json!({ "i": i })).collect();
            Ok(LoaderResult::done(items))
        });
        let ctx = LoaderContext::new("rows", AuthoredValue::object());
        let err = registry.load_records("big", scrub(ctx)).await.unwrap_err();
        assert_eq!(err.code(), "loader.result_too_large");
    }

    #[tokio::test]
    async fn load_records_rejects_cumulative_page_bytes() {
        let registry = LoaderRegistry::new().register_record("wide", |_ctx| async {
            Ok(LoaderResult::done(vec![
                Value::String("a".repeat(600_000)),
                Value::String("b".repeat(600_000)),
            ]))
        });
        let ctx = LoaderContext::new("rows", AuthoredValue::object());
        let error = registry.load_records("wide", scrub(ctx)).await.unwrap_err();
        assert_eq!(error.code(), "loader.result_too_large");
        assert!(
            error
                .params()
                .iter()
                .any(|(key, value)| key == "resource" && value == "serialized item bytes")
        );
    }

    #[tokio::test]
    async fn load_records_rejects_excessive_item_depth() {
        let registry = LoaderRegistry::new().register_record("deep", |_ctx| async {
            let mut value = Value::Null;
            for _ in 0..=64 {
                value = Value::Array(vec![value]);
            }
            Ok(LoaderResult::done(vec![value]))
        });
        let ctx = LoaderContext::new("rows", AuthoredValue::object());
        let error = registry.load_records("deep", scrub(ctx)).await.unwrap_err();
        assert_eq!(error.code(), "loader.result_too_large");
        assert!(
            error
                .params()
                .iter()
                .any(|(key, value)| key == "resource" && value == "item depth")
        );
    }

    #[test]
    fn loader_context_builder() {
        let ctx = LoaderContext::new("my_field", AuthoredValue::object())
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

    fn redacted_literal() -> AuthoredValue {
        AuthoredValue::from_data(json!(SECRET_REDACTED)).unwrap()
    }

    #[test]
    fn with_secrets_redacted_object_nested_and_non_secret_unchanged() {
        let schema = Schema::builder()
            .add(
                Field::object(field_key!("config"))
                    .add(Field::secret(field_key!("api_key")))
                    .add(Field::string(field_key!("label"))),
            )
            .build()
            .expect("valid schema");
        let values = AuthoredValue::from_data(json!({
            "config": {
                "api_key": "hunter2",
                "label": "visible"
            }
        }))
        .expect("values");
        let ctx = LoaderContext::new("k", values)
            .redacted(schema.fields())
            .unwrap();
        let config = ctx.0.values.get("config").expect("config");
        let ValueTree::Object(map) = config else {
            panic!("expected object, got {config:?}");
        };
        assert_eq!(map.get("api_key"), Some(&redacted_literal()));
        let label = map.get("label").expect("label");
        assert_eq!(label.as_str(), Some("visible"));
    }

    #[test]
    fn with_secrets_redacted_list_of_secrets() {
        let schema = Schema::builder()
            .add(Field::list(field_key!("tokens")).item(Field::secret(field_key!("t"))))
            .build()
            .expect("valid schema");
        let values = AuthoredValue::from_data(json!({ "tokens": ["a", "b"] })).expect("values");
        let ctx = LoaderContext::new("k", values)
            .redacted(schema.fields())
            .unwrap();
        let list = ctx.0.values.get("tokens").expect("tokens");
        let ValueTree::List(items) = list else {
            panic!("expected list, got {list:?}");
        };
        assert_eq!(items.as_slice(), &[redacted_literal(), redacted_literal()]);
    }

    /// Mode variant payload is an `Object` with a secret leaf and a non-secret sibling
    /// (exercises the `Field::Mode` + nested `Object` path, not a bare `Field::Secret`
    /// that replaces the entire mode `value` tree with one redacted literal).
    #[test]
    fn with_secrets_redacted_mode_variant_object_with_nested_secret() {
        let schema = Schema::builder()
            .add(
                Field::mode(field_key!("auth"))
                    .variant(
                        "oauth",
                        "OAuth",
                        Field::object(field_key!("creds"))
                            .add(Field::secret(field_key!("client_secret")))
                            .add(Field::string(field_key!("client_id"))),
                    )
                    .variant("plain", "Plain", Field::string(field_key!("name"))),
            )
            .build()
            .expect("valid schema");
        // The mode `value` is the unwrapped object payload: same shape as a top-level
        // `Object` field's value (child keys), not `{"creds": { ... }}` — see `redact` +
        // `Field::Object` matching in `redact_secrets_in_value_for_loader`.
        let values = AuthoredValue::from_data(json!({
            "auth": {
                "mode": "oauth",
                "value": {
                    "client_secret": "top",
                    "client_id": "visible"
                }
            }
        }))
        .expect("values");
        let ctx = LoaderContext::new("k", values)
            .redacted(schema.fields())
            .unwrap();
        let auth = ctx.0.values.get("auth").expect("auth");
        let ValueTree::Object(map) = auth else {
            panic!("expected object envelope, got {auth:?}");
        };
        let mode = map.get("mode").expect("mode");
        assert_eq!(mode.as_str(), Some("oauth"));
        let payload = map.get("value").expect("payload");
        let ValueTree::Object(m) = payload else {
            panic!("expected object payload, got {payload:?}");
        };
        assert_eq!(m.get("client_secret"), Some(&redacted_literal()));
        let id = m.get("client_id").expect("id");
        assert_eq!(id.as_str(), Some("visible"));
    }

    #[test]
    fn with_secrets_redacted_mode_object_without_mode_uses_default_variant() {
        let schema = Schema::builder()
            .add(
                Field::mode(field_key!("auth"))
                    .variant(
                        "oauth",
                        "OAuth",
                        Field::object(field_key!("creds"))
                            .add(Field::secret(field_key!("client_secret")))
                            .add(Field::string(field_key!("client_id"))),
                    )
                    .default_variant("oauth"),
            )
            .build()
            .expect("valid schema");
        let values = AuthoredValue::from_data(json!({
            "auth": {
                "value": {
                    "client_secret": "top",
                    "client_id": "visible"
                }
            }
        }))
        .expect("values");

        let ctx = LoaderContext::new("k", values)
            .redacted(schema.fields())
            .unwrap();
        let auth = ctx.0.values.get("auth").expect("auth");
        let ValueTree::Object(map) = auth else {
            panic!("expected object envelope, got {auth:?}");
        };
        let payload = map.get("value").expect("payload");
        let ValueTree::Object(m) = payload else {
            panic!("expected object payload, got {payload:?}");
        };
        assert_eq!(m.get("client_secret"), Some(&redacted_literal()));
        let id = m.get("client_id").expect("id");
        assert_eq!(id.as_str(), Some("visible"));
    }

    #[test]
    fn with_secrets_redacted_expression_on_secret_is_literal_token() {
        let schema = Schema::builder()
            .add(Field::secret(field_key!("api_key")))
            .build()
            .expect("valid schema");
        let mut values = AuthoredValue::object();
        values
            .insert(
                "api_key",
                ValueTree::Expression(Expression::new("would.leak()")),
            )
            .unwrap();
        let ctx = LoaderContext::new("k", values)
            .redacted(schema.fields())
            .unwrap();
        let v = ctx.0.values.get("api_key").expect("api_key");
        assert_eq!(*v, redacted_literal());
    }

    #[test]
    fn with_secrets_redacted_object_literal_blob_is_over_redacted() {
        // Objects cannot hide inside literals. Serialized secret data supplied
        // as a scalar to a secret-bearing container still needs whole redaction.
        let error = ScalarValue::try_from(json!({"api_key": "PLAINTEXT-LEAK"})).unwrap_err();
        assert_eq!(error.code(), "type_mismatch");
        let schema = Schema::builder()
            .add(
                Field::object(field_key!("cfg"))
                    .add(Field::secret(field_key!("api_key")))
                    .add(Field::string(field_key!("label"))),
            )
            .build()
            .expect("valid schema");
        let values = AuthoredValue::from_data(json!({
            "cfg": json!({"api_key": "PLAINTEXT-LEAK", "label": "x"}).to_string()
        }))
        .unwrap();
        let ctx = LoaderContext::new("k", values)
            .redacted(schema.fields())
            .unwrap();
        let cfg = ctx.0.values.get("cfg").expect("cfg");
        assert_eq!(*cfg, redacted_literal());
        assert!(
            !format!("{cfg:?}").contains("PLAINTEXT-LEAK"),
            "blob plaintext reached the loader: {cfg:?}"
        );
    }

    #[test]
    fn with_secrets_redacted_mode_unknown_variant_over_redacts() {
        // The canonical object envelope must redact an unknown variant's payload.
        let schema = Schema::builder()
            .add(Field::mode(field_key!("auth")).variant(
                "oauth",
                "OAuth",
                Field::object(field_key!("creds")).add(Field::secret(field_key!("client_secret"))),
            ))
            .build()
            .expect("valid schema");
        let values = AuthoredValue::from_data(json!({
            "auth": {"mode": "nonexistent", "value": {"client_secret": "PLAINTEXT-LEAK"}}
        }))
        .unwrap();
        let ctx = LoaderContext::new("k", values)
            .redacted(schema.fields())
            .unwrap();
        let auth = ctx.0.values.get("auth").expect("auth");
        assert_eq!(
            auth.get("mode").and_then(ValueTree::as_str),
            Some("nonexistent")
        );
        assert_eq!(auth.get("value"), Some(&redacted_literal()));
        assert!(!format!("{auth:?}").contains("PLAINTEXT-LEAK"));
    }
}
