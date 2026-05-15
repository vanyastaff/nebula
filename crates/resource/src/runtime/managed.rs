//! Per-registration runtime holding topology + metadata.
//!
//! [`ManagedResource`] is the internal representation of a registered
//! resource. It bundles the resource implementation, hot-swappable config,
//! topology runtime, release queue, and lifecycle metadata.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use arc_swap::ArcSwap;

use super::TopologyRuntime;
use crate::{
    error::Error,
    integration::AcquireResilience,
    recovery::RecoveryGate,
    release_queue::ReleaseQueue,
    resource::Resource,
    state::{ResourcePhase, ResourceStatus},
};

/// Per-registration runtime holding topology + metadata.
///
/// Created once per `Manager::register()` call and stored for the
/// lifetime of the resource. The `config` and `status` fields are
/// atomically swappable for hot-reload.
pub struct ManagedResource<R: Resource> {
    /// The resource implementation (topology trait impl).
    pub(crate) resource: R,
    /// Hot-swappable operational configuration.
    pub(crate) config: ArcSwap<R::Config>,
    /// Topology-specific runtime state.
    pub(crate) topology: TopologyRuntime<R>,
    /// Background worker pool for async cleanup.
    pub(crate) release_queue: Arc<ReleaseQueue>,
    /// Monotonically increasing generation counter (bumped on reload).
    pub(crate) generation: AtomicU64,
    /// Current lifecycle status (phase + last error).
    pub(crate) status: ArcSwap<ResourceStatus>,
    /// Optional resilience configuration (timeout + retry) for acquire.
    pub(crate) resilience: Option<AcquireResilience>,
    /// Optional recovery gate for thundering-herd prevention.
    ///
    /// When set, acquire calls check the gate before proceeding and
    /// trigger passive recovery on transient failures.
    pub(crate) recovery_gate: Option<Arc<RecoveryGate>>,
    /// Resource-level taint flag set by [`taint`](Self::taint).
    ///
    /// When `true`, the manager's acquire paths reject new acquires for
    /// this resource. Used by `Manager::revoke_slot` to stop handing out
    /// leases on a revoked credential *before* draining in-flight work and
    /// invoking the revoke hook. This is the resource-scoped analogue of
    /// the per-handle taint on [`ResourceGuard`](crate::guard::ResourceGuard)
    /// and the manager-wide `shutting_down` flag — one shared mechanism,
    /// not a parallel one.
    pub(crate) tainted: AtomicBool,
}

impl<R: Resource> ManagedResource<R> {
    /// Returns the current generation counter.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Returns a snapshot of the current lifecycle status.
    pub fn status(&self) -> Arc<ResourceStatus> {
        self.status.load_full()
    }

    /// Returns a snapshot of the current configuration.
    pub fn config(&self) -> Arc<R::Config> {
        self.config.load_full()
    }

    /// Atomically replace the lifecycle status with a new phase.
    ///
    /// Rebuilds a fresh [`ResourceStatus`] from the latest snapshot,
    /// copying the current generation across and preserving `last_error`.
    /// Used by the manager to drive phase transitions on register, reload
    /// and shutdown (#387).
    pub(crate) fn set_phase(&self, phase: ResourcePhase) {
        let prev = self.status.load_full();
        let next = ResourceStatus {
            phase,
            generation: self.generation(),
            last_error: prev.last_error.clone(),
        };
        self.status.store(Arc::new(next));
    }

    /// Replace the lifecycle status with `Failed` and record a reason.
    ///
    /// Wired by `Manager::set_phase_all_failed` (R-023): when
    /// `DrainTimeoutPolicy::Abort` fires we transition every registered
    /// resource to `Failed` so callers cannot subsequently acquire a
    /// resource the manager has already declared bankrupt. Per-resource
    /// `HealthChanged{healthy:false}` event emission is owned by the
    /// manager because it holds the broadcast channel.
    pub(crate) fn set_failed(&self, error: impl Into<String>) {
        let next = ResourceStatus {
            phase: ResourcePhase::Failed,
            generation: self.generation(),
            last_error: Some(error.into()),
        };
        self.status.store(Arc::new(next));
    }

    /// Marks the resource tainted so the manager rejects new acquires.
    ///
    /// Reuses the same "stop new leases" semantics as the per-handle
    /// `ResourceGuard::taint` and the manager-wide `shutting_down` flag —
    /// `Manager::revoke_slot` taints *before* draining so no caller can
    /// acquire a lease on the credential being revoked.
    pub(crate) fn taint(&self) {
        self.tainted.store(true, Ordering::Release);
    }

    /// Returns `true` if [`taint`](Self::taint) has been called.
    pub(crate) fn is_tainted(&self) -> bool {
        self.tainted.load(Ordering::Acquire)
    }

    /// Borrows the live runtime(s) for this topology and invokes
    /// [`Resource::on_credential_refresh`] for `slot`.
    ///
    /// Single-runtime topologies (Resident / Service / Transport /
    /// Exclusive) dispatch once against the shared runtime; Pool dispatches
    /// per idle instance. Resident before its first acquire has no runtime
    /// yet — nothing to refresh, so this is a no-op `Ok(())`.
    pub(crate) async fn dispatch_on_refresh(&self, slot: &str) -> Result<(), Error> {
        match &self.topology {
            TopologyRuntime::Resident(rt) => match rt.current() {
                Some(runtime) => self
                    .resource
                    .on_credential_refresh(slot, &runtime)
                    .await
                    .map_err(Into::into),
                None => Ok(()),
            },
            TopologyRuntime::Service(rt) => self
                .resource
                .on_credential_refresh(slot, rt.runtime())
                .await
                .map_err(Into::into),
            TopologyRuntime::Transport(rt) => self
                .resource
                .on_credential_refresh(slot, rt.runtime())
                .await
                .map_err(Into::into),
            TopologyRuntime::Exclusive(rt) => self
                .resource
                .on_credential_refresh(slot, rt.runtime())
                .await
                .map_err(Into::into),
            TopologyRuntime::Pool(rt) => rt
                .dispatch_slot_hook_over_idle(&self.resource, slot, true)
                .await
                .map_err(Into::into),
        }
    }

    /// Borrows the live runtime(s) for this topology and invokes
    /// [`Resource::on_credential_revoke`] for `slot`.
    ///
    /// Same per-topology fan-out as [`dispatch_on_refresh`](Self::dispatch_on_refresh).
    pub(crate) async fn dispatch_on_revoke(&self, slot: &str) -> Result<(), Error> {
        match &self.topology {
            TopologyRuntime::Resident(rt) => match rt.current() {
                Some(runtime) => self
                    .resource
                    .on_credential_revoke(slot, &runtime)
                    .await
                    .map_err(Into::into),
                None => Ok(()),
            },
            TopologyRuntime::Service(rt) => self
                .resource
                .on_credential_revoke(slot, rt.runtime())
                .await
                .map_err(Into::into),
            TopologyRuntime::Transport(rt) => self
                .resource
                .on_credential_revoke(slot, rt.runtime())
                .await
                .map_err(Into::into),
            TopologyRuntime::Exclusive(rt) => self
                .resource
                .on_credential_revoke(slot, rt.runtime())
                .await
                .map_err(Into::into),
            TopologyRuntime::Pool(rt) => rt
                .dispatch_slot_hook_over_idle(&self.resource, slot, false)
                .await
                .map_err(Into::into),
        }
    }
}
