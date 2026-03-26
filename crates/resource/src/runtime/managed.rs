//! Per-registration runtime holding topology + metadata.
//!
//! [`ManagedResource`] is the internal representation of a registered
//! resource. It bundles the resource implementation, hot-swappable config,
//! topology runtime, release queue, and lifecycle metadata.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;

use crate::integration::AcquireResilience;
use crate::metrics::ResourceMetrics;
use crate::release_queue::ReleaseQueue;
use crate::resource::Resource;
use crate::state::ResourceStatus;

use super::TopologyRuntime;

/// Per-registration runtime holding topology + metadata.
///
/// Created once per `Manager::register()` call and stored for the
/// lifetime of the resource. The `config` and `status` fields are
/// atomically swappable for hot-reload.
// Reason: fields used in Phase 5 (Manager) — suppress until wired.
#[allow(dead_code)]
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
    /// Per-resource metrics (independent of the aggregate counters on Manager).
    pub(crate) metrics: Arc<ResourceMetrics>,
    /// Optional resilience configuration (timeout + retry) for acquire.
    pub(crate) resilience: Option<AcquireResilience>,
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

    /// Returns the per-resource metrics for this managed resource.
    pub fn metrics(&self) -> &Arc<ResourceMetrics> {
        &self.metrics
    }
}
