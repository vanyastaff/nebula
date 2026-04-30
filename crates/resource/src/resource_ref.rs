//! Typed resource reference (`ResourceRef<R>`) — slot-binding handle for action fields.
//!
//! Per ADR-0043 §9, `ResourceRef<R>` is the canonical field type for the
//! slot-binding pattern: an action declares
//!
//! ```ignore
//! #[derive(Action)]
//! struct SendTelegram {
//!     #[resource(key = "bot")]
//!     bot: ResourceRef<TelegramBot>,   // lazy variant — see ResourceGuard for eager
//! }
//! ```
//!
//! and the framework resolves the slot per ADR-0042's binding mechanism: the
//! `key` attribute supplies a default resource id; workflow-JSON
//! `slot_bindings.<slot_key>.resource_id` overrides it per node.
//!
//! ## Phase 1 scope (current)
//!
//! `ResourceRef<R>` carries the resolved resource id and a `PhantomData<R>`
//! marker. `.resolve(ctx)` delegates to the existing `HasResources::acquire_any`
//! path (the dyn-erased accessor) and downcasts via the `ResourceGuard<R>`
//! pattern from `HasResourcesExt`. Phase 6 of
//! `m6-resource-finalization-integration-audit.md` adds engine-side typed
//! helpers (`ctx.acquire_resource_by_id::<R>(id)`); this type's `.resolve` will
//! switch over without API change.
//!
//! ## Composability
//!
//! `Option<ResourceRef<R>>` and `Lazy<ResourceRef<R>>` (from `nebula_core::Lazy`)
//! compose for optional / lazy semantics per ADR-0043 §3.

use std::{fmt, marker::PhantomData};

use nebula_core::{ResourceKey, context::HasResources};

use crate::{
    Resource, ResourceGuard,
    error::{Error, ErrorKind},
};

/// Typed reference to a registered resource.
///
/// The reference carries a resource id (slot binding per ADR-0042) and a
/// type-level marker selecting the concrete `Resource` impl whose
/// `Lease` should be acquired on resolve. The field type alone tells the
/// framework what slot kind, what concrete resource type, and (via wrapper
/// composition) whether resolution is eager / lazy / optional.
///
/// Use [`ResourceRef::resolve`] inside action / trigger bodies to obtain a
/// [`ResourceGuard<R>`] (RAII, releases on drop).
///
/// `ResourceRef<R>` is `Clone` and cheap (id + zero-sized phantom). Cloning
/// does **not** acquire a new resource lease — leases materialize only
/// during `.resolve()`.
pub struct ResourceRef<R: ?Sized> {
    /// Resolved resource id (per ADR-0042 — slot-key default OR workflow-JSON override).
    id: String,
    /// Type-level marker for the concrete `Resource` impl. `fn() -> R` so
    /// `ResourceRef<R>` is `Send + Sync` regardless of `R`.
    _phantom: PhantomData<fn() -> R>,
}

impl<R: ?Sized> ResourceRef<R> {
    /// Constructs a reference bound to the given resource id.
    ///
    /// Typically called by the macro-emitted `#[derive(Action)]` factory body
    /// — plugin authors do not write this directly. The `id` is sourced from
    /// the slot's `key` attribute or from the workflow node's `slot_bindings`
    /// override.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            _phantom: PhantomData,
        }
    }

    /// Returns the resolved resource id this reference binds to.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl<R: Resource> ResourceRef<R> {
    /// Resolves the reference to a [`ResourceGuard<R>`].
    ///
    /// Builds a [`ResourceKey`] from the carried id, calls `acquire_any` on
    /// the context's resource accessor, and downcasts to `ResourceGuard<R>`
    /// matching the existing `HasResourcesExt::resource` pattern.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Permanent`] — id cannot be coerced into a valid [`ResourceKey`], the resource
    ///   is not registered under the id, or the accessor returns a value of the wrong type (slot
    ///   type mismatch).
    pub async fn resolve<Ctx>(&self, ctx: &Ctx) -> Result<ResourceGuard<R>, Error>
    where
        Ctx: HasResources + Sync + ?Sized,
    {
        let key = ResourceKey::new(&self.id).map_err(|e| {
            Error::new(
                ErrorKind::Permanent,
                format!(
                    "resource id `{id}` is not a valid ResourceKey: {e}",
                    id = self.id
                ),
            )
        })?;

        let boxed = ctx.resources().acquire_any(&key).await.map_err(|e| {
            Error::new(ErrorKind::Permanent, e.to_string()).with_resource_key(key.clone())
        })?;

        boxed
            .downcast::<ResourceGuard<R>>()
            .map(|b| *b)
            .map_err(|_| {
                Error::new(
                    ErrorKind::Permanent,
                    format!(
                        "resource `{id}`: type mismatch (expected ResourceGuard<{ty}>)",
                        id = self.id,
                        ty = std::any::type_name::<R>(),
                    ),
                )
                .with_resource_key(key)
            })
    }
}

impl<R: ?Sized> Clone for ResourceRef<R> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<R: ?Sized> fmt::Debug for ResourceRef<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceRef")
            .field("id", &self.id)
            .field("type", &std::any::type_name::<R>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Phantom resource type for tests. The `.resolve()` path is exercised in
    // integration tests once Phase 6 wires the engine-side helpers; these
    // unit tests cover construction + accessors + Debug + Clone.
    struct FakeRes;

    #[test]
    fn new_carries_id() {
        let r: ResourceRef<FakeRes> = ResourceRef::new("main-pool");
        assert_eq!(r.id(), "main-pool");
    }

    #[test]
    fn clone_preserves_id() {
        let r: ResourceRef<FakeRes> = ResourceRef::new("primary");
        let cloned = r.clone();
        assert_eq!(r.id(), cloned.id());
    }

    #[test]
    fn debug_includes_id_and_type() {
        let r: ResourceRef<FakeRes> = ResourceRef::new("res-1");
        let s = format!("{r:?}");
        assert!(s.contains("res-1"));
        assert!(s.contains("FakeRes"));
    }
}
