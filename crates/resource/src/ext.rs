//! Typed resource access extension trait.
//!
//! Provides the ergonomic `ctx.resource::<R>().await?` *ad-hoc* surface
//! for any context implementing `HasResources`. Looks up the resource by
//! the compile-time `R::key()` constant only — no per-node id binding.
//!
//! # When to use which surface
//!
//! Two surfaces resolve resources in actions; pick by binding shape:
//!
//! 1. **Slot binding** (preferred for production code) — declare an `#[resource(key = "...")]`
//!    field on the action struct; the `#[derive(Action)]` macro emits a `FromWorkflowNode` factory
//!    that calls `ActionContextExt::acquire_resource_by_id` (in `nebula-action`) with the per-node
//!    binding from [slot binding]().
//!    Per-node overrides via workflow JSON `node.slot_bindings` are honored automatically.
//!
//! 2. **`ctx.resource::<R>()` ad-hoc** (this module) — type-only lookup by `R::key()`. No per-node
//!    binding, no slot definition required. Useful for actions that don't declare a slot, generic
//!    helpers, and one-off resource probes. Does not pick up `node.slot_bindings` overrides — the
//!    caller decides which type they want, the engine returns whatever matches `R::key()` in the
//!    layered scope.
//!
//! Both paths route through the same `LayeredResourceAccessor` (in
//! `nebula-engine::scoped_resources`) injected into the action context, so the `scoped → global`
//! precedence applies uniformly.
//!
//! # Examples
//!
//! Slot-binding form (preferred). Shown as `text`, not a compiled doctest: the
//! `#[derive(Action)]` macro, `StatelessAction`, and `ActionContext` live in
//! `nebula-action`, which depends on this crate, so they cannot be named here:
//!
//! ```text
//! #[derive(Action)]
//! #[action(key = "send.report", input = SendReportInput, output = ReportId)]
//! struct SendReport {
//!     #[resource(key = "db")] db: ResourceGuard<Postgres>,
//! }
//!
//! impl StatelessAction for SendReport {
//!     async fn execute(&self, input: SendReportInput, _ctx: &impl ActionContext)
//!         -> Result<ActionResult<ReportId>, ActionError>
//!     {
//!         // self.db already resolved by FromWorkflowNode factory.
//!         let id = self.db.insert_report(&input).await?;
//!         Ok(ActionResult::ok(id))
//!     }
//! }
//! ```
//!
//! Ad-hoc form — `no_run` (acquisition needs a live engine accessor + runtime),
//! but it type-checks against the real API: `ctx.resource::<R>()` looks the
//! resource up by `R::key()` only, for any context implementing `HasResources`:
//!
//! ```rust,no_run
//! use nebula_resource::{Error, HasResourcesExt, Provider, ResourceGuard};
//!
//! async fn fetch<R, Ctx>(ctx: &Ctx) -> Result<ResourceGuard<R>, Error>
//! where
//!     R: Provider,
//!     Ctx: HasResourcesExt,
//! {
//!     // Type-only lookup — no per-node slot binding.
//!     ctx.resource::<R>().await
//! }
//! ```

use std::future::Future;

use nebula_core::context::HasResources;

use crate::{
    Provider, ResourceGuard,
    error::{Error, guard_type_mismatch},
};

/// Sealing module — keeps [`HasResourcesExt`] closed to external impls.
///
/// `HasResourcesExt` adds a method *over* every `HasResources` via a
/// blanket impl. Without sealing, a downstream crate could write its
/// own `impl HasResourcesExt for SomeOtherType` and collide with the
/// blanket — a published-library semver hazard (rust-intel §C1). The
/// sealed supertrait makes external impls a compile error; the blanket
/// impl below is the sole impl path, and adding methods to
/// `HasResourcesExt` is therefore non-breaking.
mod sealed {
    pub trait Sealed {}
    impl<C: super::HasResources + ?Sized> Sealed for C {}
}

/// Typed resource access for any context implementing `HasResources`.
///
/// Ad-hoc form: looks up by `R::key()` only. For per-node slot binding,
/// declare an `#[resource]` field on the action struct and let the
/// derive-emitted `FromWorkflowNode` factory call
/// `ActionContextExt::acquire_resource_by_id` instead — slot binding is
/// the preferred path for production code (see crate-level docs).
///
/// **Sealed.** External crates cannot implement this trait; the sole
/// impl is the crate-internal blanket `impl<C: HasResources + ?Sized>
/// HasResourcesExt for C`. New methods can be added here without
/// breaking downstream code.
///
/// **Call through a concrete context, not `&dyn`.** Both methods carry a
/// `where Self: Sized` bound (their `impl Future` returns require a sized
/// receiver pre-AFIT), so `ctx.resource::<R>()` is *not* callable on a
/// `&dyn Context`/`&dyn ActionContext` even though the blanket impl covers
/// `?Sized` types. Hold the concrete context type (the action body already
/// receives `&impl ActionContext`) when reaching for this surface.
///
/// # Examples
///
/// `no_run` (acquisition needs a live accessor + runtime), but type-checked
/// against the real API. Call through a concrete `HasResources` context — not
/// `&dyn` — as the note above requires:
///
/// ```rust,no_run
/// use nebula_resource::{Error, HasResourcesExt, Provider, ResourceGuard};
///
/// async fn acquire<Db, Cache, Ctx>(ctx: &Ctx) -> Result<(), Error>
/// where
///     Db: Provider,
///     Cache: Provider,
///     Ctx: HasResourcesExt,
/// {
///     let _pool: ResourceGuard<Db> = ctx.resource::<Db>().await?;
///     let _cache: Option<ResourceGuard<Cache>> = ctx.try_resource::<Cache>().await?;
///     Ok(())
/// }
/// ```
pub trait HasResourcesExt: HasResources + sealed::Sealed {
    /// Acquire a typed resource guard. Returns error if not found or acquisition fails.
    fn resource<R: Provider>(&self) -> impl Future<Output = Result<ResourceGuard<R>, Error>> + Send
    where
        Self: Sized;

    /// Try to acquire a typed resource guard. Returns `Ok(None)` if the resource
    /// is not registered, `Err` if registered but acquisition fails.
    fn try_resource<R: Provider>(
        &self,
    ) -> impl Future<Output = Result<Option<ResourceGuard<R>>, Error>> + Send
    where
        Self: Sized;
}

impl<C: HasResources + ?Sized> HasResourcesExt for C {
    async fn resource<R: Provider>(&self) -> Result<ResourceGuard<R>, Error>
    where
        Self: Sized,
    {
        let key = R::key();
        // Preserve the retryable/`retry_after` classification carried by the
        // accessor-seam `CoreError` (see `impl From<CoreError> for Error`);
        // re-wrapping as `Permanent` would silently disable retries for a
        // transient acquire failure.
        let boxed = self
            .resources()
            .acquire_any(&key)
            .await
            .map_err(|e| Error::from(e).with_resource_key(key.clone()))?;

        boxed
            .downcast::<ResourceGuard<R>>()
            .map(|b| *b)
            .map_err(|_| guard_type_mismatch::<R>(key))
    }

    async fn try_resource<R: Provider>(&self) -> Result<Option<ResourceGuard<R>>, Error>
    where
        Self: Sized,
    {
        let key = R::key();
        match self.resources().try_acquire_any(&key).await {
            Ok(Some(boxed)) => {
                let guard = boxed
                    .downcast::<ResourceGuard<R>>()
                    .map(|b| *b)
                    .map_err(|_| guard_type_mismatch::<R>(key.clone()))?;
                Ok(Some(guard))
            },
            Ok(None) => Ok(None),
            // Preserve the accessor-seam retryable classification (see
            // `impl From<CoreError> for Error`).
            Err(e) => Err(Error::from(e).with_resource_key(key)),
        }
    }
}
