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
//! precedence (Phase 6 / M6.1) applies uniformly.
//!
//! # Examples
//!
//! Slot-binding form (preferred):
//!
//! ```rust,ignore
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
//! Ad-hoc form:
//!
//! ```rust,ignore
//! use nebula_resource::HasResourcesExt;
//!
//! async fn write_audit(ctx: &impl ActionContext, line: &str) -> Result<(), ActionError> {
//!     // Type-only lookup — searches by AuditLog::key() only.
//!     let log = ctx.resource::<AuditLog>().await?;
//!     log.append(line).await?;
//!     Ok(())
//! }
//! ```

use std::future::Future;

use nebula_core::context::HasResources;

use crate::{
    Resource, ResourceGuard,
    error::{Error, ErrorKind},
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
/// # Examples
///
/// ```ignore
/// use nebula_resource::HasResourcesExt;
///
/// let pool = ctx.resource::<PostgresResource>().await?;
/// let cache = ctx.try_resource::<RedisResource>().await?;
/// ```
pub trait HasResourcesExt: HasResources + sealed::Sealed {
    /// Acquire a typed resource guard. Returns error if not found or acquisition fails.
    fn resource<R: Resource>(&self) -> impl Future<Output = Result<ResourceGuard<R>, Error>> + Send
    where
        Self: Sized;

    /// Try to acquire a typed resource guard. Returns `Ok(None)` if the resource
    /// is not registered, `Err` if registered but acquisition fails.
    fn try_resource<R: Resource>(
        &self,
    ) -> impl Future<Output = Result<Option<ResourceGuard<R>>, Error>> + Send
    where
        Self: Sized;
}

impl<C: HasResources + ?Sized> HasResourcesExt for C {
    async fn resource<R: Resource>(&self) -> Result<ResourceGuard<R>, Error>
    where
        Self: Sized,
    {
        let key = R::key();
        let boxed = self.resources().acquire_any(&key).await.map_err(|e| {
            Error::new(ErrorKind::Permanent, e.to_string()).with_resource_key(key.clone())
        })?;

        boxed
            .downcast::<ResourceGuard<R>>()
            .map(|b| *b)
            .map_err(|_| {
                Error::new(
                    ErrorKind::Permanent,
                    format!(
                        "resource type mismatch: expected ResourceGuard<{}> for key `{key}`",
                        std::any::type_name::<R>(),
                    ),
                )
                .with_resource_key(key)
            })
    }

    async fn try_resource<R: Resource>(&self) -> Result<Option<ResourceGuard<R>>, Error>
    where
        Self: Sized,
    {
        let key = R::key();
        match self.resources().try_acquire_any(&key).await {
            Ok(Some(boxed)) => {
                let guard = boxed
                    .downcast::<ResourceGuard<R>>()
                    .map(|b| *b)
                    .map_err(|_| {
                        Error::new(
                            ErrorKind::Permanent,
                            format!(
                                "resource type mismatch: expected ResourceGuard<{}> for key `{key}`",
                                std::any::type_name::<R>(),
                            ),
                        )
                        .with_resource_key(key.clone())
                    })?;
                Ok(Some(guard))
            },
            Ok(None) => Ok(None),
            Err(e) => Err(Error::new(ErrorKind::Permanent, e.to_string()).with_resource_key(key)),
        }
    }
}
