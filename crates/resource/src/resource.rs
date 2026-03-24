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

/// Synchronous broken-check result, returned by [`Resource::is_broken`].
///
/// `Broken` causes the pool to discard the instance immediately and skip the
/// async recycle round-trip.  `Healthy` lets the normal recycle path proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokenCheck {
    /// Instance appears healthy — proceed with async recycle.
    Healthy,
    /// Instance is permanently broken — discard immediately, no recycle.
    Broken,
}

impl BrokenCheck {
    /// Returns `true` when the check result is `Broken`.
    #[must_use]
    pub fn is_broken(self) -> bool {
        matches!(self, Self::Broken)
    }
}

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
    ///
    /// `meta` carries pool lifecycle context (age, idle time, acquire count)
    /// so implementations can make metadata-aware decisions without coupling
    /// the instance type to pool internals.
    fn is_reusable(
        &self,
        _instance: &Self::Instance,
        _meta: &InstanceMetadata,
    ) -> impl Future<Output = Result<bool>> + Send {
        async { Ok(true) }
    }

    /// Synchronous O(1) check for obvious breakage, called during Guard release.
    ///
    /// Invoked synchronously in the drop path before the async recycle round-trip.
    /// Must not block. Checks cheap indicators: closed TCP socket, invalid descriptor,
    /// or an atomic broken flag set by the instance.
    ///
    /// [`BrokenCheck::Broken`] → pool discards the instance and spawns async destroy (no recycle).
    /// [`BrokenCheck::Healthy`] → pool continues to the normal async recycle path.
    ///
    /// Complements [`is_reusable`](Self::is_reusable): `is_broken` is a fast release-time
    /// guard; `is_reusable` is a deeper validity check at acquire-from-idle time.
    fn is_broken(&self, _instance: &Self::Instance) -> BrokenCheck {
        BrokenCheck::Healthy
    }

    /// Recycle an instance before returning it to the pool.
    ///
    /// `meta` carries pool lifecycle context (age, idle time, acquire count)
    /// for implementations that need metadata-aware recycling decisions.
    fn recycle(
        &self,
        _instance: &mut Self::Instance,
        _meta: &InstanceMetadata,
    ) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
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

    /// Permanently destroy an instance when it is removed from the pool.
    ///
    /// Called whenever an instance is discarded: on pool shutdown, after a failed
    /// recycle, or when it exceeds its maximum lifetime. The default is a no-op
    /// (the instance is simply dropped).
    fn destroy(&self, _instance: Self::Instance) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
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
