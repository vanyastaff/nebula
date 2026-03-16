//! Core resource traits (bb8-style).
//!
//! The `Resource` trait defines how to create, validate, recycle, and clean up
//! resource instances. Each resource type exposes a **canonical [`ResourceKey`]**
//! used across manager, events, errors, and metrics.

use std::any::Any;
use std::future::Future;

use nebula_core::ResourceKey;

use crate::context::Context;
use crate::error::Result;
use crate::metadata::ResourceMetadata;
use crate::pool::InstanceMetadata;

/// Configuration trait for resource types.
pub trait Config: Send + Sync + 'static {
    /// Validate the configuration, returning an error if invalid.
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

/// Core resource trait (bb8-style).
///
/// Defines the full lifecycle: create, validate, recycle, cleanup.
/// Each resource type has an associated `Config`, `Instance`, and a canonical
/// key that is the single source of truth for manager indexing, events, and metrics.
pub trait Resource: Send + Sync + 'static {
    /// The configuration type for this resource.
    type Config: Config;

    /// The instance type produced by this resource.
    type Instance: Send + Sync + 'static;

    /// Canonical key identifying this resource type.
    ///
    /// This is the **single source of truth** for the resource's identity.
    /// The manager indexes pools by this key; events and metrics use it too.
    fn key(&self) -> ResourceKey;

    /// Static metadata (display name, description, tags, icon) for UI and discovery.
    ///
    /// Defaults to a minimal [`ResourceMetadata`] derived from [`key()`](Self::key).
    /// Override to provide richer metadata (display name, description, tags, icon).
    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(self.key())
    }

    /// Create a new instance from config and context.
    fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> impl Future<Output = Result<Self::Instance>> + Send;

    /// Check whether an existing instance can be safely reused.
    ///
    /// Return `Ok(true)` when the instance is reusable, `Ok(false)` when it
    /// should be discarded, and `Err(_)` on validation failure.
    fn is_reusable(&self, _instance: &Self::Instance) -> impl Future<Output = Result<bool>> + Send {
        async { Ok(true) }
    }

    /// Synchronous O(1) check for obvious breakage, called during Guard release.
    ///
    /// Invoked synchronously in the drop path before the async recycle round-trip.
    /// Must not block. Checks cheap indicators: closed TCP socket, invalid descriptor,
    /// or an atomic broken flag set by the instance.
    ///
    /// `true`  → pool discards the instance and spawns async cleanup (no recycle).
    /// `false` → pool continues to the normal async recycle path.
    ///
    /// Complements [`is_reusable`](Self::is_reusable): `is_broken` is a fast release-time
    /// guard; `is_reusable` is a deeper validity check at acquire-from-idle time.
    fn is_broken(&self, _instance: &Self::Instance) -> bool {
        false
    }

    /// Recycle an instance before returning it to the pool.
    fn recycle(&self, _instance: &mut Self::Instance) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Deep reuse check with pool lifecycle metadata.
    ///
    /// Same contract as [`is_reusable`](Self::is_reusable) but also receives
    /// [`InstanceMetadata`] so that adapters can make decisions based on instance
    /// age, idle time, or acquisition count without storing that data inside the
    /// instance itself.
    ///
    /// **Migration note:** Override this method instead of `is_reusable` when you
    /// need metadata.  The pool calls this first; `is_reusable` is only called when
    /// this default implementation is not overridden (it delegates to `is_reusable`).
    fn is_reusable_with_meta(
        &self,
        instance: &Self::Instance,
        _meta: &InstanceMetadata,
    ) -> impl Future<Output = Result<bool>> + Send {
        self.is_reusable(instance)
    }

    /// Recycle with pool lifecycle metadata.
    ///
    /// Same as [`recycle`](Self::recycle) but also receives [`InstanceMetadata`] so
    /// that adapters can make fine-grained decisions (e.g. skip flush if the instance
    /// was idle for less than a second). Defaults to delegating to `recycle`.
    ///
    /// **Migration note:** Override this method instead of `recycle` when you need
    /// the metadata; do not override both.
    fn recycle_with_meta(
        &self,
        instance: &mut Self::Instance,
        _meta: &InstanceMetadata,
    ) -> impl Future<Output = Result<()>> + Send {
        self.recycle(instance)
    }

    /// Prepare an instance for a specific execution context before handing it to the caller.
    ///
    /// Called after a successful [`recycle`](Self::recycle) and immediately before the
    /// [`Guard`](crate::guard::Guard) is returned to the caller. Receives the full
    /// [`Context`] with `execution_id`, `workflow_id`, `tenant_id`, and scope.
    ///
    /// Typical uses:
    /// - **Logger**: set structured fields (`execution_id`, `workflow_id`, `tenant_id`).
    /// - **HTTP client**: inject a `X-Correlation-ID` into default headers.
    /// - **Telegram bot**: bind a `correlation_id` to the rate-limiter context.
    ///
    /// The counterpart is [`recycle`](Self::recycle): `prepare` sets execution-scoped
    /// fields, `recycle` clears them so the instance is returned "clean" to the idle queue.
    ///
    /// Default: no-op. Zero overhead on the hot path when not overridden.
    fn prepare(
        &self,
        _instance: &mut Self::Instance,
        _ctx: &Context,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Clean up an instance when it is permanently removed.
    fn cleanup(&self, instance: Self::Instance) -> impl Future<Output = Result<()>> + Send {
        async {
            drop(instance);
            Ok(())
        }
    }
}

/// Object-safe supertrait for declaring resource dependencies.
///
/// `Resource` and `Action` return `Vec<Box<dyn AnyResource>>` to declare
/// "I need resources of these types." The engine uses `Any::type_id()` on
/// `dyn AnyResource` to identify the resource type at registration time.
///
/// Automatically implemented for all `R: Resource` via the blanket impl below.
pub trait AnyResource: Any + Send + Sync + 'static {
    /// The normalized key identifying this resource type.
    fn resource_key(&self) -> ResourceKey;

    /// Metadata for this resource type.
    fn resource_metadata(&self) -> ResourceMetadata;
}

/// Blanket impl: every `Resource` is automatically an `AnyResource`.
impl<R: Resource + 'static> AnyResource for R {
    fn resource_key(&self) -> ResourceKey {
        self.key()
    }

    fn resource_metadata(&self) -> ResourceMetadata {
        self.metadata()
    }
}

/// Declarative dependency declaration for resources.
///
/// Implement this trait on a `Resource` type to declare which
/// sub-resource dependencies it requires. The engine calls these methods
/// at registration time to build the dependency graph automatically.
///
/// Methods use `where Self: Sized` so they are not in the vtable and can
/// only be called on concrete types at registration time.
pub trait ResourceDependencies {
    /// Sub-resources required by this resource.
    fn resources() -> Vec<Box<dyn AnyResource>>
    where
        Self: Sized,
    {
        vec![]
    }
}

/// Blanket impl: every `Resource` automatically satisfies `ResourceDependencies`
/// with an empty dependency list. Override by adding an explicit
/// `impl ResourceDependencies for MyResource { fn resources() -> … }`.
impl<T: Resource> ResourceDependencies for T {}
