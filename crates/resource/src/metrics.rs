//! Registry-backed counters for resource operation tracking.
//!
//! [`ResourceOpsMetrics`] wraps a fixed set of [`Counter`]s and
//! [`Histogram`]s from a shared [`MetricsRegistry`], replacing the
//! previous hand-rolled atomic counters. Use
//! [`snapshot()`](ResourceOpsMetrics::snapshot) to capture a
//! point-in-time view as [`ResourceOpsSnapshot`].
//!
//! Rotation-dispatch metrics (`rotation_attempts_total`,
//! `revoke_attempts_total`, `rotation_dispatch_latency_seconds`) are
//! pre-bound at construction with the closed `rotation_outcome` label set
//! (see `nebula_metrics::naming::rotation_outcome`) so the hot path on
//! `Manager::on_credential_refreshed` / `_revoked` is a label-free atomic
//! increment / observe — no registry lookup, no allocation per dispatch.

use nebula_metrics::naming::{
    NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL, NEBULA_RESOURCE_ACQUIRE_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_DISPATCH_LATENCY_SECONDS,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_SKIPPED_TOTAL, NEBULA_RESOURCE_DESTROY_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL, rotation_outcome,
};
use nebula_telemetry::metrics::{Counter, Histogram, MetricsRegistry};

use crate::error::{RefreshOutcome, RevokeOutcome};

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
    /// Rotation-attempt counters pre-bound by closed `outcome` label.
    rotation_attempts: OutcomeBoundCounters,
    /// Revoke-attempt counters pre-bound by closed `outcome` label.
    revoke_attempts: OutcomeBoundCounters,
    /// Rotation dispatch latency histograms pre-bound by closed `outcome`
    /// label. Wraps the full per-dispatcher span (`SchemeFactory::acquire`
    /// + resource hook) inside the per-resource timeout budget.
    rotation_dispatch_latency: OutcomeBoundHistograms,
    /// Counter for dispatchers skipped during fan-out because the
    /// dispatcher's `scheme_type_id()` did not match the caller's
    /// credential type. Unlabeled — a non-zero crossing is a register-
    /// side bug surface and the per-resource error log carries the
    /// triage detail.
    credential_rotation_skipped: Counter,
}

impl ResourceOpsMetrics {
    /// Creates a new metrics instance backed by the given registry.
    ///
    /// Counters are registered (or retrieved if already present) using the
    /// standard naming constants from `nebula-metrics`. Rotation/revoke
    /// dispatch series are pre-bound by `outcome` label so the hot-path
    /// observation is a single atomic operation.
    #[must_use]
    pub fn new(registry: &MetricsRegistry) -> Self {
        Self {
            acquire_total: registry.counter(NEBULA_RESOURCE_ACQUIRE_TOTAL),
            acquire_errors: registry.counter(NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL),
            release_total: registry.counter(NEBULA_RESOURCE_RELEASE_TOTAL),
            create_total: registry.counter(NEBULA_RESOURCE_CREATE_TOTAL),
            destroy_total: registry.counter(NEBULA_RESOURCE_DESTROY_TOTAL),
            rotation_attempts: OutcomeBoundCounters::new(
                registry,
                NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
            ),
            revoke_attempts: OutcomeBoundCounters::new(
                registry,
                NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
            ),
            rotation_dispatch_latency: OutcomeBoundHistograms::new(
                registry,
                NEBULA_RESOURCE_CREDENTIAL_ROTATION_DISPATCH_LATENCY_SECONDS,
            ),
            credential_rotation_skipped: registry
                .counter(NEBULA_RESOURCE_CREDENTIAL_ROTATION_SKIPPED_TOTAL),
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

    /// Records a single per-resource refresh dispatch.
    ///
    /// Increments the rotation-attempts counter labeled by `outcome` and
    /// observes `elapsed_seconds` into the rotation-dispatch-latency
    /// histogram with the same label.
    pub fn record_rotation_dispatch(&self, outcome: &RefreshOutcome, elapsed_seconds: f64) {
        let label = refresh_outcome_label(outcome);
        self.rotation_attempts.inc(label);
        self.rotation_dispatch_latency
            .observe(label, elapsed_seconds);
    }

    /// Records a single per-resource revoke dispatch.
    ///
    /// Increments the revoke-attempts counter labeled by `outcome` and
    /// observes `elapsed_seconds` into the rotation-dispatch-latency
    /// histogram with the same label. The latency histogram is shared
    /// across refresh and revoke because both flow through the same
    /// `dispatcher.dispatch_*` + `tokio::time::timeout` shell.
    pub fn record_revoke_dispatch(&self, outcome: &RevokeOutcome, elapsed_seconds: f64) {
        let label = revoke_outcome_label(outcome);
        self.revoke_attempts.inc(label);
        self.rotation_dispatch_latency
            .observe(label, elapsed_seconds);
    }

    /// Records a dispatcher skipped during fan-out because its
    /// `scheme_type_id()` did not match the caller's credential type.
    ///
    /// Mirrors the per-resource `tracing::error!` emitted at the same
    /// site so operators see a metric crossing in addition to the log.
    /// A non-zero value is a register-side bug and a real signal — pair
    /// with the log lines to identify which `(resource_key, credential_id)`
    /// pair drifted.
    pub fn record_rotation_skipped(&self) {
        self.credential_rotation_skipped.inc();
    }

    /// Captures a point-in-time snapshot of all counters.
    ///
    /// Each counter is read with [`Relaxed`](std::sync::atomic::Ordering::Relaxed)
    /// ordering. The snapshot is not atomic across all fields — concurrent
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
            rotation_attempts: self.rotation_attempts.snapshot(),
            revoke_attempts: self.revoke_attempts.snapshot(),
            rotation_dispatch_latency_count: self.rotation_dispatch_latency.snapshot_count(),
            credential_rotation_skipped: self.credential_rotation_skipped.get(),
        }
    }
}

/// Per-`outcome` counter handles for a single rotation/revoke series.
///
/// Pre-binds three counters (one per `rotation_outcome` label, see
/// `nebula_metrics::naming::rotation_outcome`) at registration time so the
/// hot path is `inc()` only — no registry lookup, no allocation per
/// dispatch.
#[derive(Debug, Clone)]
struct OutcomeBoundCounters {
    success: Counter,
    failed: Counter,
    timed_out: Counter,
}

impl OutcomeBoundCounters {
    fn new(registry: &MetricsRegistry, name: &str) -> Self {
        let interner = registry.interner();
        let label_success = interner.label_set(&[("outcome", rotation_outcome::SUCCESS)]);
        let label_failed = interner.label_set(&[("outcome", rotation_outcome::FAILED)]);
        let label_timed_out = interner.label_set(&[("outcome", rotation_outcome::TIMED_OUT)]);
        Self {
            success: registry.counter_labeled(name, &label_success),
            failed: registry.counter_labeled(name, &label_failed),
            timed_out: registry.counter_labeled(name, &label_timed_out),
        }
    }

    fn inc(&self, outcome_label: &'static str) {
        match outcome_label {
            rotation_outcome::SUCCESS => self.success.inc(),
            rotation_outcome::FAILED => self.failed.inc(),
            rotation_outcome::TIMED_OUT => self.timed_out.inc(),
            _ => debug_assert!(false, "unknown rotation outcome label: {outcome_label}"),
        }
    }

    fn snapshot(&self) -> OutcomeCountersSnapshot {
        OutcomeCountersSnapshot {
            success: self.success.get(),
            failed: self.failed.get(),
            timed_out: self.timed_out.get(),
        }
    }
}

/// Per-`outcome` histogram handles for the rotation-dispatch latency series.
#[derive(Debug, Clone)]
struct OutcomeBoundHistograms {
    success: Histogram,
    failed: Histogram,
    timed_out: Histogram,
}

impl OutcomeBoundHistograms {
    fn new(registry: &MetricsRegistry, name: &str) -> Self {
        let interner = registry.interner();
        let label_success = interner.label_set(&[("outcome", rotation_outcome::SUCCESS)]);
        let label_failed = interner.label_set(&[("outcome", rotation_outcome::FAILED)]);
        let label_timed_out = interner.label_set(&[("outcome", rotation_outcome::TIMED_OUT)]);
        Self {
            success: registry.histogram_labeled(name, &label_success),
            failed: registry.histogram_labeled(name, &label_failed),
            timed_out: registry.histogram_labeled(name, &label_timed_out),
        }
    }

    fn observe(&self, outcome_label: &'static str, value: f64) {
        match outcome_label {
            rotation_outcome::SUCCESS => self.success.observe(value),
            rotation_outcome::FAILED => self.failed.observe(value),
            rotation_outcome::TIMED_OUT => self.timed_out.observe(value),
            _ => debug_assert!(false, "unknown rotation outcome label: {outcome_label}"),
        }
    }

    fn snapshot_count(&self) -> OutcomeCountersSnapshot {
        OutcomeCountersSnapshot {
            success: self.success.count() as u64,
            failed: self.failed.count() as u64,
            timed_out: self.timed_out.count() as u64,
        }
    }
}

fn refresh_outcome_label(outcome: &RefreshOutcome) -> &'static str {
    match outcome {
        RefreshOutcome::Ok => rotation_outcome::SUCCESS,
        RefreshOutcome::Failed(_) => rotation_outcome::FAILED,
        RefreshOutcome::TimedOut { .. } => rotation_outcome::TIMED_OUT,
    }
}

fn revoke_outcome_label(outcome: &RevokeOutcome) -> &'static str {
    match outcome {
        RevokeOutcome::Ok => rotation_outcome::SUCCESS,
        RevokeOutcome::Failed(_) => rotation_outcome::FAILED,
        RevokeOutcome::TimedOut { .. } => rotation_outcome::TIMED_OUT,
    }
}

/// Per-`outcome` counter snapshot. Mirrors the
/// `nebula_metrics::naming::rotation_outcome` closed label set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutcomeCountersSnapshot {
    /// Resources that completed the dispatch hook with `Ok(())`.
    pub success: u64,
    /// Resources whose hook returned `Err`.
    pub failed: u64,
    /// Resources whose hook exceeded the per-resource timeout budget.
    pub timed_out: u64,
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
    /// Per-`outcome` rotation-attempt counts.
    pub rotation_attempts: OutcomeCountersSnapshot,
    /// Per-`outcome` revoke-attempt counts.
    pub revoke_attempts: OutcomeCountersSnapshot,
    /// Per-`outcome` rotation-dispatch latency observation counts.
    /// Sums and percentiles are read directly from the registry.
    pub rotation_dispatch_latency_count: OutcomeCountersSnapshot,
    /// Total dispatchers skipped during rotation/revoke fan-out due to a
    /// `scheme_type_id()` mismatch with the caller's credential type. A
    /// non-zero crossing is a register-side bug.
    pub credential_rotation_skipped: u64,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::error::Error;

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
        assert_eq!(snap.rotation_attempts, OutcomeCountersSnapshot::default());
        assert_eq!(snap.revoke_attempts, OutcomeCountersSnapshot::default());
        assert_eq!(
            snap.rotation_dispatch_latency_count,
            OutcomeCountersSnapshot::default()
        );
        assert_eq!(snap.credential_rotation_skipped, 0);
    }

    #[test]
    fn rotation_skipped_counter_increments() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);

        metrics.record_rotation_skipped();
        metrics.record_rotation_skipped();
        metrics.record_rotation_skipped();

        assert_eq!(metrics.snapshot().credential_rotation_skipped, 3);
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

    #[test]
    fn rotation_dispatch_observes_outcome_labels() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);

        metrics.record_rotation_dispatch(&RefreshOutcome::Ok, 0.012);
        metrics.record_rotation_dispatch(&RefreshOutcome::Ok, 0.025);
        metrics.record_rotation_dispatch(&RefreshOutcome::Failed(Error::permanent("boom")), 0.040);
        metrics.record_rotation_dispatch(
            &RefreshOutcome::TimedOut {
                budget: Duration::from_secs(1),
            },
            1.0,
        );

        let snap = metrics.snapshot();
        assert_eq!(snap.rotation_attempts.success, 2);
        assert_eq!(snap.rotation_attempts.failed, 1);
        assert_eq!(snap.rotation_attempts.timed_out, 1);
        assert_eq!(snap.rotation_dispatch_latency_count.success, 2);
        assert_eq!(snap.rotation_dispatch_latency_count.failed, 1);
        assert_eq!(snap.rotation_dispatch_latency_count.timed_out, 1);
        // Revoke counters untouched.
        assert_eq!(snap.revoke_attempts, OutcomeCountersSnapshot::default());
    }

    #[test]
    fn revoke_dispatch_observes_outcome_labels() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry);

        metrics.record_revoke_dispatch(&RevokeOutcome::Ok, 0.010);
        metrics.record_revoke_dispatch(&RevokeOutcome::Failed(Error::permanent("boom")), 0.030);
        metrics.record_revoke_dispatch(
            &RevokeOutcome::TimedOut {
                budget: Duration::from_secs(2),
            },
            2.0,
        );
        metrics.record_revoke_dispatch(
            &RevokeOutcome::TimedOut {
                budget: Duration::from_secs(2),
            },
            2.0,
        );

        let snap = metrics.snapshot();
        assert_eq!(snap.revoke_attempts.success, 1);
        assert_eq!(snap.revoke_attempts.failed, 1);
        assert_eq!(snap.revoke_attempts.timed_out, 2);
        // Latency histogram is shared across refresh + revoke.
        assert_eq!(snap.rotation_dispatch_latency_count.success, 1);
        assert_eq!(snap.rotation_dispatch_latency_count.failed, 1);
        assert_eq!(snap.rotation_dispatch_latency_count.timed_out, 2);
        // Rotation counters untouched.
        assert_eq!(snap.rotation_attempts, OutcomeCountersSnapshot::default());
    }
}
