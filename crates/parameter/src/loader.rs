//! Dynamic loader types for select, filter-field, and dynamic-record fields.
//!
//! A loader is an async function attached directly to the field that produced it.
//! The engine resolves credentials and injects them via [`LoaderContext`], then
//! calls the loader to populate options, filter fields, or field specs at runtime.
//!
//! Loaders are **not serialized** — they live only on the in-process
//! [`Parameter`](crate::parameter::Parameter) value returned by `action.metadata()`.

use std::{future::Future, pin::Pin, sync::Arc};

use crate::{filter_field::FilterField, loader_result::LoaderResult, option::SelectOption};

/// Boxed future returned by loader closures.
pub type LoaderFuture<T> = Pin<Box<dyn Future<Output = Result<T, LoaderError>> + Send>>;

// ── LoaderError ─────────────────────────────────────────────────────────────

/// Error returned by a loader when it cannot resolve data.
///
/// This is intentionally a simple struct rather than a categorised enum —
/// the parameter crate does not model transport-layer concerns. Action
/// authors create `LoaderError` with the appropriate message.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LoaderError {
    /// Human-readable description of the failure.
    pub message: String,
    /// Optional underlying cause for error chaining.
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl LoaderError {
    /// Creates a loader error with a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Creates a loader error wrapping a source error.
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

// ── LoaderContext ────────────────────────────────────────────────────────────

/// Context passed to loader functions when the UI requests dynamic data.
///
/// Renamed from `LoaderCtx` in v2 for clarity.
#[derive(Clone)]
pub struct LoaderContext {
    /// The id of the field requesting a load.
    pub field_id: String,
    /// Current parameter values at the time of the request.
    pub values: serde_json::Value,
    /// Optional text filter entered by the user (for searchable selects).
    pub filter: Option<String>,
    /// Pagination cursor returned from a previous load.
    pub cursor: Option<String>,
    /// Resolved credential value, engine-populated.
    ///
    /// Supplied as opaque JSON so the parameter crate stays decoupled from
    /// `nebula-credential`.
    pub credential: Option<serde_json::Value>,
    /// Additional loader-specific metadata.
    ///
    /// Actions can attach arbitrary JSON here to pass extra context to loaders
    /// (e.g. API endpoint configuration, feature flags).
    pub metadata: Option<serde_json::Value>,
}

impl std::fmt::Debug for LoaderContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoaderContext")
            .field("field_id", &self.field_id)
            .field("values", &self.values)
            .field("filter", &self.filter)
            .field("cursor", &self.cursor)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "<redacted>"),
            )
            .field("metadata", &self.metadata)
            .finish()
    }
}

/// Backwards-compatible alias for [`LoaderContext`].
///
/// Prefer `LoaderContext` in new code.
#[deprecated(since = "0.3.0", note = "renamed to `LoaderContext`")]
pub type LoaderCtx = LoaderContext;

// ── Generic Loader ──────────────────────────────────────────────────────────

/// Generic async loader that resolves items of type `T` for a parameter field.
///
/// The engine resolves credentials and injects them via [`LoaderContext`], then
/// calls the loader to populate data at runtime.
///
/// Two loaders always compare equal (`PartialEq` returns `true`), so
/// adding a loader does not affect schema equality checks.
pub struct Loader<T: Send + 'static>(
    Arc<dyn Fn(LoaderContext) -> LoaderFuture<LoaderResult<T>> + Send + Sync>,
);

impl<T: Send + 'static> Loader<T> {
    /// Wraps an async closure as a [`Loader`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(LoaderContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<LoaderResult<T>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader with the given context.
    ///
    /// # Errors
    ///
    /// Returns [`LoaderError`] if the loader cannot resolve data.
    pub async fn call(&self, ctx: LoaderContext) -> Result<LoaderResult<T>, LoaderError> {
        (self.0)(ctx).await
    }
}

impl<T: Send + 'static> Clone for Loader<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Send + 'static> PartialEq for Loader<T> {
    /// Always returns `true` — loaders are not compared structurally.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T: Send + 'static> std::fmt::Debug for Loader<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Loader(<async fn>)")
    }
}

/// Async loader that resolves [`SelectOption`]s for Select/MultiSelect fields.
pub type OptionLoader = Loader<SelectOption>;

/// Async loader that resolves JSON records for Dynamic fields.
pub type RecordLoader = Loader<serde_json::Value>;

/// Async loader that resolves [`FilterField`]s for Filter fields.
pub type FilterFieldLoader = Loader<FilterField>;
