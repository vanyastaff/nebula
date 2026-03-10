//! Dynamic loader types for select and dynamic-record fields.
//!
//! A loader is a plain function attached directly to the field that produced it.
//! The engine resolves credentials and injects them via [`LoaderCtx`], then
//! calls the loader to populate options or field specs at runtime.
//!
//! Loaders are **not serialized** — they live only on the in-process
//! [`crate::field::Field`] value returned by `action.metadata()`.

use std::sync::Arc;

use crate::option::SelectOption;
use crate::runtime::ParameterValues;
use crate::spec::FieldSpec;

// ── LoaderCtx ────────────────────────────────────────────────────────────────

/// Context passed to loader functions when the UI requests dynamic data.
#[derive(Debug, Clone)]
pub struct LoaderCtx {
    /// The id of the field requesting a load.
    pub field_id: String,
    /// Current parameter values at the time of the request.
    pub values: ParameterValues,
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

/// Inline loader that resolves [`SelectOption`]s for a [`crate::field::Field::Select`]
/// or [`FieldSpec::Select`] field with [`crate::option::OptionSource::Dynamic`].
///
/// Two [`OptionLoader`]s always compare equal (`PartialEq` returns `true`),
/// so adding a loader does not affect schema equality checks.
pub struct OptionLoader(pub Arc<dyn Fn(&LoaderCtx) -> Vec<SelectOption> + Send + Sync>);

impl OptionLoader {
    /// Wraps a closure as an [`OptionLoader`].
    pub fn new(f: impl Fn(&LoaderCtx) -> Vec<SelectOption> + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Invokes the loader with the given context.
    pub fn call(&self, ctx: &LoaderCtx) -> Vec<SelectOption> {
        (self.0)(ctx)
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
        f.write_str("OptionLoader(<fn>)")
    }
}

// ── RecordLoader ─────────────────────────────────────────────────────────────

/// Inline loader that resolves [`FieldSpec`]s for a
/// [`crate::field::Field::DynamicRecord`] field.
///
/// Like [`OptionLoader`], two [`RecordLoader`]s always compare equal.
pub struct RecordLoader(pub Arc<dyn Fn(&LoaderCtx) -> Vec<FieldSpec> + Send + Sync>);

impl RecordLoader {
    /// Wraps a closure as a [`RecordLoader`].
    pub fn new(f: impl Fn(&LoaderCtx) -> Vec<FieldSpec> + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Invokes the loader with the given context.
    pub fn call(&self, ctx: &LoaderCtx) -> Vec<FieldSpec> {
        (self.0)(ctx)
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
        f.write_str("RecordLoader(<fn>)")
    }
}
