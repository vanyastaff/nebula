//! Registry-backed counters for resource operation tracking.
//!
//! [`ResourceOpsMetrics`] wraps five [`Counter`]s from a shared
//! [`MetricsRegistry`], replacing the previous hand-rolled atomic
//! counters. Use [`snapshot()`](ResourceOpsMetrics::snapshot) to
//! capture a point-in-time view as [`ResourceOpsSnapshot`].

use nebula_metrics::naming::{
    NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL, NEBULA_RESOURCE_ACQUIRE_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_DESTROY_TOTAL, NEBULA_RESOURCE_RELEASE_TOTAL,
};
use nebula_telemetry::metrics::{Counter, MetricsRegistry};

/// Registry-backed counters for resource operations.
///
/// Each counter is a [`Clone`]-cheap handle into the shared
/// [`MetricsRegistry`]. Multiple clones of the same
/// `ResourceOpsMetrics` increment the same underlying atomic.
///
/// # Examples
///
/// ```
/// use nebula_resource::metrics::ResourceOpsMetrics;
/// use nebula_telemetry::metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let metrics = ResourceOpsMetrics::new(&registry);
/// metrics.record_acquire();
/// metrics.record_acquire();
/// metrics.record_acquire_error();
///
/// let snap = metrics.snapshot();
/// assert_eq!(snap.acquire_total, 2);
/// assert_eq!(snap.acquire_errors, 1);
/// ```
#[derive(Debug, Clone)]
pub struct ResourceOpsMetrics {
    acquire_total: Counter,
    acquire_errors: Counter,
    release_total: Counter,
    create_total: Counter,
    destroy_total: Counter,
}

impl ResourceOpsMetrics {
    /// Creates a new metrics instance backed by the given registry.
    ///
    /// Counters are registered (or retrieved if already present) using the
    /// standard naming constants from `nebula-metrics`.
    #[must_use]
    pub fn new(registry: &MetricsRegistry) -> Self {
        Self {
            acquire_total: registry.counter(NEBULA_RESOURCE_ACQUIRE_TOTAL),
            acquire_errors: registry.counter(NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL),
            release_total: registry.counter(NEBULA_RESOURCE_RELEASE_TOTAL),
            create_total: registry.counter(NEBULA_RESOURCE_CREATE_TOTAL),
            destroy_total: registry.counter(NEBULA_RESOURCE_DESTROY_TOTAL),
        }
    }

    /// Records a successful acquire.
    pub fn record_acquire(&self) {
        self.acquire_total.inc();
    }

    /// Records a failed acquire attempt.
    pub fn record_acquire_error(&self) {
        self.acquire_errors.inc();
    }

    /// Records a release (handle drop).
    pub fn record_release(&self) {
        self.release_total.inc();
    }

    /// Records a new resource instance creation.
    pub fn record_create(&self) {
        self.create_total.inc();
    }

    /// Records a resource instance destruction.
    pub fn record_destroy(&self) {
        self.destroy_total.inc();
    }

    /// Captures a point-in-time snapshot of all counters.
    ///
    /// Each counter is read with [`Relaxed`](std::sync::atomic::Ordering::Relaxed)
    /// ordering. The snapshot is not atomic across all five fields — concurrent
    /// increments may be observed in any combination. This is acceptable for
    /// best-effort telemetry.
    #[must_use]
    pub fn snapshot(&self) -> ResourceOpsSnapshot {
        ResourceOpsSnapshot {
            acquire_total: self.acquire_total.get(),
            acquire_errors: self.acquire_errors.get(),
            release_total: self.release_total.get(),
            create_total: self.create_total.get(),
            destroy_total: self.destroy_total.get(),
        }
    }
}

/// Point-in-time snapshot of resource operation counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceOpsSnapshot {
    /// Total successful acquires.
    pub acquire_total: u64,
    /// Total failed acquire attempts.
    pub acquire_errors: u64,
    /// Total releases (handle drops).
    pub release_total: u64,
    /// Total resource instances created.
    pub create_total: u64,
    /// Total resource instances destroyed.
    pub destroy_total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);
        let snap = metrics.snapshot();
        assert_eq!(snap.acquire_total, 0);
        assert_eq!(snap.acquire_errors, 0);
        assert_eq!(snap.release_total, 0);
        assert_eq!(snap.create_total, 0);
        assert_eq!(snap.destroy_total, 0);
    }

    #[test]
    fn record_and_snapshot() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);
        metrics.record_acquire();
        metrics.record_acquire();
        metrics.record_acquire_error();
        metrics.record_release();
        metrics.record_create();
        metrics.record_create();
        metrics.record_create();
        metrics.record_destroy();

        let snap = metrics.snapshot();
        assert_eq!(snap.acquire_total, 2);
        assert_eq!(snap.acquire_errors, 1);
        assert_eq!(snap.release_total, 1);
        assert_eq!(snap.create_total, 3);
        assert_eq!(snap.destroy_total, 1);
    }

    #[test]
    fn clones_share_counters() {
        let registry = MetricsRegistry::new();
        let m1 = ResourceOpsMetrics::new(&registry);
        let m2 = m1.clone();

        m1.record_acquire();
        m2.record_acquire();

        assert_eq!(m1.snapshot().acquire_total, 2);
        assert_eq!(m2.snapshot().acquire_total, 2);
    }

    #[test]
    fn backed_by_registry() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);
        metrics.record_create();
        metrics.record_create();

        // Read directly from registry to verify shared backing.
        let counter = registry.counter(NEBULA_RESOURCE_CREATE_TOTAL);
        assert_eq!(counter.get(), 2);
    }
}
