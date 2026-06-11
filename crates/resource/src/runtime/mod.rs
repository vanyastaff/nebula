//! Topology runtime implementations.
//!
//! Each topology trait ([`Pooled`], [`Resident`]) has a corresponding runtime
//! struct that manages instance lifecycle, and a dispatch enum
//! ([`TopologyKind`]) that erases the topology at the registration level.
//! [`TopologyRuntime`] wraps the kind enum and carries a function pointer
//! built at construction time (where topology bounds are in scope) so that
//! [`ManagedHandle::acquire`](crate::registry::ManagedHandle::acquire) can
//! dispatch to the correct pipeline without carrying topology trait bounds on
//! the `dyn`-safe trait.
//!
//! [`Pooled`]: crate::topology::pooled::Pooled
//! [`Resident`]: crate::topology::resident::Resident

pub mod managed;
pub mod pool;
pub mod resident;

use std::{any::Any, future::Future, pin::Pin, sync::Arc};

use crate::{
    context::ResourceContext, error::Error, options::AcquireOptions, resource::Provider,
    topology_tag::TopologyTag,
};

/// Variant enum for topology runtime state.
///
/// Carries the runtime-specific state for each topology. Match on
/// `topology.kind` to access the variant data.
pub enum TopologyKind<R: Provider> {
    /// Pool of N interchangeable instances with checkout/recycle.
    Pool(pool::PoolRuntime<R>),
    /// Single shared instance, clone on acquire.
    Resident(resident::ResidentRuntime<R>),
}

impl<R: Provider> TopologyKind<R> {
    /// Returns the [`TopologyTag`] for this variant.
    pub fn tag(&self) -> TopologyTag {
        match self {
            Self::Pool(_) => TopologyTag::Pool,
            Self::Resident(_) => TopologyTag::Resident,
        }
    }
}

/// Boxed future returned by every acquire-dispatch path.
pub(crate) type AcquireBoxFuture =
    Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, Error>> + Send + 'static>>;

/// Type alias for the private acquire dispatch function pointer.
///
/// A function pointer (not a closure) that dispatches the topology-specific
/// acquire pipeline. Built at [`TopologyRuntime`] construction time where
/// the topology trait bounds (`R: Resident` / `R: Pooled`) are in scope;
/// stored so that [`ManagedHandle::acquire`](crate::registry::ManagedHandle::acquire)
/// can dispatch without carrying those bounds on the `dyn`-safe trait.
pub(crate) type AcquireDispatch<R> = fn(
    Arc<managed::ManagedResource<R>>,
    Arc<crate::manager::Manager>,
    ResourceContext,
    AcquireOptions,
) -> Pin<
    Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, Error>> + Send + 'static>,
>;

/// Topology runtime: kind enum + acquire dispatch function pointer.
///
/// Built once per `Manager::register()` call. The `acquire` field is a
/// zero-overhead function pointer (not a heap-allocated closure) that
/// monomorphizes the correct pipeline for this resource's topology.
pub struct TopologyRuntime<R: Provider> {
    /// The topology variant and its runtime state.
    pub(crate) kind: TopologyKind<R>,
    /// Zero-cost function pointer to the topology-specific acquire pipeline.
    pub(crate) acquire: AcquireDispatch<R>,
}

impl<R: Provider> TopologyRuntime<R> {
    /// Returns the topology tag for this runtime.
    pub fn tag(&self) -> TopologyTag {
        self.kind.tag()
    }

    /// Dispatches the topology-specific acquire pipeline.
    ///
    /// Called by [`ManagedHandle::acquire`](crate::registry::ManagedHandle::acquire).
    pub(crate) fn dispatch_acquire(
        &self,
        managed: Arc<managed::ManagedResource<R>>,
        mgr: Arc<crate::manager::Manager>,
        ctx: ResourceContext,
        opts: AcquireOptions,
    ) -> AcquireBoxFuture {
        (self.acquire)(managed, mgr, ctx, opts)
    }
}

// ─── Resident constructor ────────────────────────────────────────────────────

use crate::{resource::HasCredentialSlots, topology::resident::Resident};

impl<R> TopologyRuntime<R>
where
    R: Resident + HasCredentialSlots + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    /// Builds a [`TopologyRuntime`] for a [`Resident`] resource.
    ///
    /// The acquire dispatch function pointer is set here — where `R: Resident`
    /// is in scope — so `ManagedHandle::acquire` does not need topology bounds.
    pub fn resident(rt: resident::ResidentRuntime<R>) -> Self {
        Self {
            kind: TopologyKind::Resident(rt),
            acquire: resident_acquire_dispatch::<R>,
        }
    }
}

fn resident_acquire_dispatch<R>(
    managed: Arc<managed::ManagedResource<R>>,
    mgr: Arc<crate::manager::Manager>,
    ctx: ResourceContext,
    opts: AcquireOptions,
) -> AcquireBoxFuture
where
    R: Resident + HasCredentialSlots + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    Box::pin(async move {
        let guard = mgr.resident_pipeline(managed, &ctx, &opts).await?;
        Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
    })
}

// ─── Pooled constructor ──────────────────────────────────────────────────────

use crate::topology::pooled::Pooled;

impl<R> TopologyRuntime<R>
where
    R: Pooled + HasCredentialSlots + Clone + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    /// Builds a [`TopologyRuntime`] for a [`Pooled`] resource.
    ///
    /// The acquire dispatch function pointer is set here — where `R: Pooled`
    /// is in scope — so `ManagedHandle::acquire` does not need topology bounds.
    pub fn pooled(rt: pool::PoolRuntime<R>) -> Self {
        Self {
            kind: TopologyKind::Pool(rt),
            acquire: pooled_acquire_dispatch::<R>,
        }
    }
}

fn pooled_acquire_dispatch<R>(
    managed: Arc<managed::ManagedResource<R>>,
    mgr: Arc<crate::manager::Manager>,
    ctx: ResourceContext,
    opts: AcquireOptions,
) -> AcquireBoxFuture
where
    R: Pooled + HasCredentialSlots + Clone + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    Box::pin(async move {
        let guard = mgr.pooled_pipeline(managed, &ctx, &opts).await?;
        Ok(Box::new(guard) as Box<dyn Any + Send + Sync>)
    })
}
