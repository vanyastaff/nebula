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
pub type LoaderFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

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
        Fut: Future<Output = Vec<SelectOption>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader with the given context.
    pub async fn call(&self, ctx: LoaderCtx) -> Vec<SelectOption> {
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
        Fut: Future<Output = Vec<FieldSpec>> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(f(ctx))))
    }

    /// Invokes the loader with the given context.
    pub async fn call(&self, ctx: LoaderCtx) -> Vec<FieldSpec> {
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
