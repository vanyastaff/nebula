//! Atomic counters for resource operation tracking.
//!
//! [`ResourceMetrics`] provides lock-free counters for acquire, release,
//! create, and destroy operations. Use [`snapshot()`](ResourceMetrics::snapshot)
//! to capture a consistent point-in-time view.

use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic counters for resource operations.
///
/// All counters use [`Relaxed`](std::sync::atomic::Ordering::Relaxed) ordering.
/// These are advisory monotonic counters — there is no ordering guarantee
/// between individual fields in a [`snapshot`](Self::snapshot), and reads may
/// observe slightly stale values on weakly-ordered architectures. This is
/// intentional: the overhead of stronger ordering is not justified for
/// fire-and-forget telemetry counters.
///
/// # Examples
///
/// ```
/// use nebula_resource::metrics::ResourceMetrics;
///
/// let metrics = ResourceMetrics::new();
/// metrics.record_acquire();
/// metrics.record_acquire();
/// metrics.record_acquire_error();
///
/// let snap = metrics.snapshot();
/// assert_eq!(snap.acquire_total, 2);
/// assert_eq!(snap.acquire_errors, 1);
/// ```
pub struct ResourceMetrics {
    acquire_total: AtomicU64,
    acquire_errors: AtomicU64,
    release_total: AtomicU64,
    create_total: AtomicU64,
    destroy_total: AtomicU64,
}

impl ResourceMetrics {
    /// Creates a new metrics instance with all counters at zero.
    pub fn new() -> Self {
        Self {
            acquire_total: AtomicU64::new(0),
            acquire_errors: AtomicU64::new(0),
            release_total: AtomicU64::new(0),
            create_total: AtomicU64::new(0),
            destroy_total: AtomicU64::new(0),
        }
    }

    /// Records a successful acquire.
    pub fn record_acquire(&self) {
        self.acquire_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a failed acquire attempt.
    pub fn record_acquire_error(&self) {
        self.acquire_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a release (handle drop).
    pub fn record_release(&self) {
        self.release_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a new resource instance creation.
    pub fn record_create(&self) {
        self.create_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a resource instance destruction.
    pub fn record_destroy(&self) {
        self.destroy_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Captures a point-in-time snapshot of all counters.
    ///
    /// Each counter is read with [`Relaxed`](std::sync::atomic::Ordering::Relaxed)
    /// ordering. The snapshot is not atomic across all five fields — concurrent
    /// increments may be observed in any combination. This is acceptable for
    /// best-effort telemetry.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            acquire_total: self.acquire_total.load(Ordering::Relaxed),
            acquire_errors: self.acquire_errors.load(Ordering::Relaxed),
            release_total: self.release_total.load(Ordering::Relaxed),
            create_total: self.create_total.load(Ordering::Relaxed),
            destroy_total: self.destroy_total.load(Ordering::Relaxed),
        }
    }
}

impl Default for ResourceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ResourceMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snap = self.snapshot();
        f.debug_struct("ResourceMetrics")
            .field("acquire_total", &snap.acquire_total)
            .field("acquire_errors", &snap.acquire_errors)
            .field("release_total", &snap.release_total)
            .field("create_total", &snap.create_total)
            .field("destroy_total", &snap.destroy_total)
            .finish()
    }
}

/// Point-in-time snapshot of resource metrics counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsSnapshot {
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
