//! Dynamic loader types for select and dynamic-record fields.
//!
//! A loader is an async function attached directly to the field that produced it.
//! The engine resolves credentials and injects them via [`LoaderCtx`], then
//! calls the loader to populate options or field specs at runtime.
//!
//! Loaders are **not serialized** — they live only on the in-process
//! [`crate::field::Field`] value returned by `action.metadata()`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::option::SelectOption;
use crate::runtime::FieldValues;
use crate::spec::FieldSpec;

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

// ── LoaderCtx ────────────────────────────────────────────────────────────────

/// Context passed to loader functions when the UI requests dynamic data.
#[derive(Debug, Clone)]
pub struct LoaderCtx {
    /// The id of the field requesting a load.
    pub field_id: String,
    /// Current parameter values at the time of the request.
    pub values: FieldValues,
    /// Optional text filter entered by the user (for searchable selects).
    pub filter: Option<String>,
    /// Pagination cursor returned from a previous load.
    pub cursor: Option<String>,
    /// Resolved credential value, engine-populated.
    ///
    /// Supplied as opaque JSON so the parameter crate stays decoupled from
    /// `nebula-credential`.
    pub credential: Option<serde_json::Value>,
}

// ── OptionLoader ─────────────────────────────────────────────────────────────

/// Async inline loader that resolves [`SelectOption`]s for a
/// [`crate::field::Field::Select`] or [`FieldSpec::Select`] field with a
/// [`crate::option::OptionSource::Dynamic`] source.
///
/// Two [`OptionLoader`]s always compare equal (`PartialEq` returns `true`),
/// so adding a loader does not affect schema equality checks.
pub struct OptionLoader(Arc<dyn Fn(LoaderCtx) -> LoaderFuture<Vec<SelectOption>> + Send + Sync>);

impl OptionLoader {
    /// Wraps an async closure as an [`OptionLoader`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(LoaderCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<SelectOption>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader with the given context.
    ///
    /// # Errors
    ///
    /// Returns [`LoaderError`] if the loader cannot resolve options.
    pub async fn call(&self, ctx: LoaderCtx) -> Result<Vec<SelectOption>, LoaderError> {
        (self.0)(ctx).await
    }
}

impl Clone for OptionLoader {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl PartialEq for OptionLoader {
    /// Always returns `true` — loaders are not compared structurally.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for OptionLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OptionLoader(<async fn>)")
    }
}

// ── RecordLoader ─────────────────────────────────────────────────────────────

/// Async inline loader that resolves [`FieldSpec`]s for a
/// [`crate::field::Field::DynamicFields`] field.
///
/// Like [`OptionLoader`], two [`RecordLoader`]s always compare equal.
pub struct RecordLoader(Arc<dyn Fn(LoaderCtx) -> LoaderFuture<Vec<FieldSpec>> + Send + Sync>);

impl RecordLoader {
    /// Wraps an async closure as a [`RecordLoader`].
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: Fn(LoaderCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Vec<FieldSpec>, LoaderError>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader with the given context.
    ///
    /// # Errors
    ///
    /// Returns [`LoaderError`] if the loader cannot resolve field specs.
    pub async fn call(&self, ctx: LoaderCtx) -> Result<Vec<FieldSpec>, LoaderError> {
        (self.0)(ctx).await
    }
}

impl Clone for RecordLoader {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl PartialEq for RecordLoader {
    /// Always returns `true` — loaders are not compared structurally.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for RecordLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecordLoader(<async fn>)")
    }
}
