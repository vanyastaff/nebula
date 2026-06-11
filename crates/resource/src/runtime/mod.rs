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
    context::ResourceContext,
    error::Error,
    options::AcquireOptions,
    resource::Provider,
    topology::{AdmissionPhase, Load, Unavailable},
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

/// Function pointer type for the sync admission-phase query.
///
/// Takes `&TopologyKind<R>` (the live runtime), returns the current
/// [`AdmissionPhase`]. Called by [`ManagedHandle`](crate::registry::ManagedHandle)
/// without topology trait bounds.
pub(crate) type AdmissionPhaseDispatch<R> = fn(&TopologyKind<R>) -> AdmissionPhase;

/// Function pointer type for the sync `try_reserve` gate.
///
/// Returns `Ok(())` if a ticket could be taken (capacity available), or
/// `Err(Unavailable)` describing why not. A dummy `InstanceStore<()>` is
/// passed because the built-in topology impls (Pool/Resident) use
/// `type Slot = ()` and do not inspect the store in these sync methods.
pub(crate) type TryReserveGateDispatch<R> = fn(&TopologyKind<R>) -> Result<(), Unavailable>;

/// Function pointer type for the sync load query.
pub(crate) type LoadDispatch<R> = fn(&TopologyKind<R>) -> Option<Load>;

/// Topology runtime: kind enum + acquire/admission dispatch function pointers.
///
/// Built once per `Manager::register()` call. The function pointer fields are
/// zero-overhead (not heap-allocated closures) that monomorphize the correct
/// pipeline for this resource's topology at construction time (where the
/// topology trait bounds are in scope).
pub struct TopologyRuntime<R: Provider> {
    /// The topology variant and its runtime state.
    pub(crate) kind: TopologyKind<R>,
    /// Zero-cost function pointer to the topology-specific acquire pipeline.
    pub(crate) acquire: AcquireDispatch<R>,
    /// Zero-cost function pointer to the sync admission-phase query.
    pub(crate) admission_phase: AdmissionPhaseDispatch<R>,
    /// Zero-cost function pointer to the sync `try_reserve` capacity gate.
    pub(crate) try_reserve_gate: TryReserveGateDispatch<R>,
    /// Zero-cost function pointer to the sync load query.
    pub(crate) load: LoadDispatch<R>,
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

    /// Returns the current admission phase without topology trait bounds.
    ///
    /// Called by [`ManagedHandle::admission_phase`](crate::registry::ManagedHandle::admission_phase).
    pub(crate) fn dispatch_admission_phase(&self) -> AdmissionPhase {
        (self.admission_phase)(&self.kind)
    }

    /// Checks whether a ticket can be taken right now (sync capacity gate).
    ///
    /// `Ok(())` = capacity available; `Err(Unavailable)` = reason for denial.
    /// Called by [`ManagedHandle::try_reserve_gate`](crate::registry::ManagedHandle::try_reserve_gate).
    pub(crate) fn dispatch_try_reserve_gate(&self) -> Result<(), Unavailable> {
        (self.try_reserve_gate)(&self.kind)
    }

    /// Returns the current load snapshot without topology trait bounds.
    pub(crate) fn dispatch_load(&self) -> Option<Load> {
        (self.load)(&self.kind)
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
    /// All dispatch function pointers are set here — where `R: Resident` is in
    /// scope — so [`ManagedHandle`](crate::registry::ManagedHandle) methods do
    /// not need topology trait bounds.
    pub fn resident(rt: resident::ResidentRuntime<R>) -> Self {
        Self {
            kind: TopologyKind::Resident(rt),
            acquire: resident_acquire_dispatch::<R>,
            admission_phase: resident_admission_phase::<R>,
            try_reserve_gate: resident_try_reserve_gate::<R>,
            load: resident_load::<R>,
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

fn resident_admission_phase<R>(kind: &TopologyKind<R>) -> AdmissionPhase
where
    R: Resident + HasCredentialSlots + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    use crate::topology::{InstanceStore, Topology as _};
    let TopologyKind::Resident(rt) = kind else {
        // Invariant: this fn pointer is only stored in Resident TopologyRuntime.
        return AdmissionPhase::Ready;
    };
    rt.phase(&InstanceStore::new(None))
}

fn resident_try_reserve_gate<R>(kind: &TopologyKind<R>) -> Result<(), Unavailable>
where
    R: Resident + HasCredentialSlots + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    use crate::topology::{InstanceStore, Topology as _};
    let TopologyKind::Resident(rt) = kind else {
        return Ok(());
    };
    // Ticket is consumed; we only need the Ok/Err signal.
    rt.try_reserve(&InstanceStore::new(None)).map(|_ticket| ())
}

fn resident_load<R>(kind: &TopologyKind<R>) -> Option<Load>
where
    R: Resident + HasCredentialSlots + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    use crate::topology::{InstanceStore, Topology as _};
    let TopologyKind::Resident(rt) = kind else {
        return None;
    };
    rt.load(&InstanceStore::new(None))
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
    /// All dispatch function pointers are set here — where `R: Pooled` is in
    /// scope — so [`ManagedHandle`](crate::registry::ManagedHandle) methods do
    /// not need topology trait bounds.
    pub fn pooled(rt: pool::PoolRuntime<R>) -> Self {
        Self {
            kind: TopologyKind::Pool(rt),
            acquire: pooled_acquire_dispatch::<R>,
            admission_phase: pooled_admission_phase::<R>,
            try_reserve_gate: pooled_try_reserve_gate::<R>,
            load: pooled_load::<R>,
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

fn pooled_admission_phase<R>(kind: &TopologyKind<R>) -> AdmissionPhase
where
    R: Pooled + HasCredentialSlots + Clone + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    use crate::topology::{InstanceStore, Topology as _};
    let TopologyKind::Pool(rt) = kind else {
        // Invariant: this fn pointer is only stored in Pool TopologyRuntime.
        return AdmissionPhase::Ready;
    };
    rt.phase(&InstanceStore::new(None))
}

fn pooled_try_reserve_gate<R>(kind: &TopologyKind<R>) -> Result<(), Unavailable>
where
    R: Pooled + HasCredentialSlots + Clone + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    use crate::topology::{InstanceStore, Topology as _};
    let TopologyKind::Pool(rt) = kind else {
        return Ok(());
    };
    rt.try_reserve(&InstanceStore::new(None)).map(|_ticket| ())
}

fn pooled_load<R>(kind: &TopologyKind<R>) -> Option<Load>
where
    R: Pooled + HasCredentialSlots + Clone + Send + Sync + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    use crate::topology::{InstanceStore, Topology as _};
    let TopologyKind::Pool(rt) = kind else {
        return None;
    };
    rt.load(&InstanceStore::new(None))
}
