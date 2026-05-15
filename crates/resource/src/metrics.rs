//! Registry-backed counters for resource operation tracking.
//!
//! [`ResourceOpsMetrics`] wraps a fixed set of [`Counter`]s from a shared
//! [`MetricsRegistry`], replacing the previous hand-rolled atomic
//! counters. Use [`snapshot()`](ResourceOpsMetrics::snapshot) to capture a
//! point-in-time view as [`ResourceOpsSnapshot`].
//!
//! Per ADR-0044 the credential rotation/revoke counters that the previous
//! `Resource::Credential` model owned have been removed. The per-slot
//! rotation hook shape is `on_credential_refresh(&self, slot_name, runtime)`
//! / `on_credential_revoke(&self, slot_name, runtime)`.
//!
//! Refresh/revoke dispatch is observed by **two distinct, non-overlapping**
//! signals fed from `Manager::{refresh_slot,revoke_slot}`:
//!
//! - **Attempt counters** (unlabeled): incremented exactly once per dispatch
//!   regardless of outcome — every call to `dispatch_*_erased` bumps these.
//! - **Outcome split** (`success`/`failed`/`timed_out`, see
//!   [`OutcomeCountersSnapshot`]): incremented once per *result* from the
//!   `Ok`/`Err`/drain-timeout arms.
//!
//! They are *not* two overlapping totals of the same thing: an attempt is the
//! act of dispatching; an outcome is how that one dispatch resolved. For a
//! refresh path with no early skip, `attempts == success + failed`. The
//! revoke path additionally records `timed_out` when the bounded in-flight
//! drain expires before the hook runs (the hook still runs afterwards, so a
//! `timed_out` revoke also records its eventual `success`/`failed`).

use nebula_metrics::{Counter, MetricsRegistry};
use nebula_metrics::{
    MetricsResult,
    naming::{
        NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL, NEBULA_RESOURCE_ACQUIRE_TOTAL,
        NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL, NEBULA_RESOURCE_DESTROY_TOTAL,
        NEBULA_RESOURCE_RELEASE_TOTAL,
    },
};

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
/// use nebula_metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let metrics = ResourceOpsMetrics::new(&registry).unwrap();
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
    slot_refresh_attempts: Counter,
    slot_revoke_attempts: Counter,
    slot_refresh_outcomes: OutcomeCounters,
    slot_revoke_outcomes: OutcomeCounters,
}

/// How a single per-slot dispatch resolved.
///
/// Closed set mirroring `nebula_metrics::naming::rotation_outcome` — one
/// value is recorded per dispatch *result* (distinct from the unlabeled
/// attempt counter, which counts every dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotDispatchOutcome {
    /// Hook returned `Ok(())`.
    Success,
    /// Hook returned `Err`.
    Failed,
    /// Bounded in-flight drain elapsed before the hook ran (`revoke_slot`).
    TimedOut,
}

/// Process-local outcome split for one dispatch direction.
///
/// Backed by standalone [`Counter`]s (not registry-bound): the unlabeled
/// `*_ATTEMPTS_TOTAL` registry counters carry the aggregate signal, while
/// this per-`outcome` breakdown is exposed through
/// [`OutcomeCountersSnapshot`] for in-process inspection. `Clone` is cheap
/// (each `Counter` is an `Arc` handle), so clones share the same atomics.
#[derive(Debug, Clone, Default)]
struct OutcomeCounters {
    success: Counter,
    failed: Counter,
    timed_out: Counter,
}

impl OutcomeCounters {
    fn record(&self, outcome: SlotDispatchOutcome) {
        match outcome {
            SlotDispatchOutcome::Success => self.success.inc(),
            SlotDispatchOutcome::Failed => self.failed.inc(),
            SlotDispatchOutcome::TimedOut => self.timed_out.inc(),
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

impl ResourceOpsMetrics {
    /// Creates a new metrics instance backed by the given registry.
    ///
    /// Counters are registered (or retrieved if already present) using the
    /// standard naming constants from `nebula-metrics`.
    pub fn new(registry: &MetricsRegistry) -> MetricsResult<Self> {
        Ok(Self {
            acquire_total: registry.counter(NEBULA_RESOURCE_ACQUIRE_TOTAL)?,
            acquire_errors: registry.counter(NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL)?,
            release_total: registry.counter(NEBULA_RESOURCE_RELEASE_TOTAL)?,
            create_total: registry.counter(NEBULA_RESOURCE_CREATE_TOTAL)?,
            destroy_total: registry.counter(NEBULA_RESOURCE_DESTROY_TOTAL)?,
            slot_refresh_attempts: registry
                .counter(NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL)?,
            slot_revoke_attempts: registry
                .counter(NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL)?,
            slot_refresh_outcomes: OutcomeCounters::default(),
            slot_revoke_outcomes: OutcomeCounters::default(),
        })
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

    /// Records one per-slot credential **refresh** dispatch attempt
    /// (`Manager::refresh_slot`), regardless of hook outcome.
    ///
    /// This is the unlabeled *attempt* counter: it counts every dispatch.
    /// The per-`outcome` breakdown is a separate, non-overlapping signal —
    /// call [`record_slot_refresh_outcome`](Self::record_slot_refresh_outcome)
    /// once the result is known. `ResourceEvent::SlotRefreshFailed`
    /// remains the eventing surface for failure correlation.
    pub fn record_slot_refresh(&self) {
        self.slot_refresh_attempts.inc();
    }

    /// Records one per-slot credential **revoke** dispatch attempt
    /// (`Manager::revoke_slot`), regardless of hook outcome. Same
    /// attempt-only semantics as [`record_slot_refresh`](Self::record_slot_refresh).
    pub fn record_slot_revoke(&self) {
        self.slot_revoke_attempts.inc();
    }

    /// Records how one `Manager::refresh_slot` dispatch resolved.
    ///
    /// Distinct from [`record_slot_refresh`](Self::record_slot_refresh):
    /// that counts the *attempt*, this partitions by *result*. With no
    /// early skip, `slot_refresh_attempts == success + failed`.
    pub fn record_slot_refresh_outcome(&self, outcome: SlotDispatchOutcome) {
        self.slot_refresh_outcomes.record(outcome);
    }

    /// Records how one `Manager::revoke_slot` dispatch resolved.
    ///
    /// Same attempt-vs-outcome relationship as
    /// [`record_slot_refresh_outcome`](Self::record_slot_refresh_outcome).
    /// `TimedOut` is recorded when the bounded in-flight drain expires
    /// before the hook runs; the hook still runs afterwards, so a
    /// timed-out revoke also records its eventual `Success`/`Failed`.
    pub fn record_slot_revoke_outcome(&self, outcome: SlotDispatchOutcome) {
        self.slot_revoke_outcomes.record(outcome);
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
            slot_refresh_attempts: self.slot_refresh_attempts.get(),
            slot_revoke_attempts: self.slot_revoke_attempts.get(),
            slot_refresh_outcomes: self.slot_refresh_outcomes.snapshot(),
            slot_revoke_outcomes: self.slot_revoke_outcomes.snapshot(),
        }
    }
}

/// Per-`outcome` counter snapshot. Mirrors the
/// `nebula_metrics::naming::rotation_outcome` closed label set.
///
/// `Manager::{refresh_slot,revoke_slot}` feed this split from their
/// `Ok` / `Err` / drain-timeout arms — one increment per dispatch
/// *result*. It is a distinct signal from the unlabeled
/// `*_ATTEMPTS_TOTAL` counters (one increment per dispatch *attempt*),
/// not a second total of the same events: with no early skip,
/// `attempts == success + failed`.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// Total per-slot credential refresh dispatch *attempts*
    /// (`Manager::refresh_slot`), unlabeled — one per dispatch. The
    /// per-result breakdown is `slot_refresh_outcomes`.
    pub slot_refresh_attempts: u64,
    /// Total per-slot credential revoke dispatch *attempts*
    /// (`Manager::revoke_slot`), unlabeled — one per dispatch.
    pub slot_revoke_attempts: u64,
    /// Per-`outcome` split of refresh dispatches (one per *result*).
    /// Distinct from `slot_refresh_attempts`; see
    /// [`OutcomeCountersSnapshot`].
    pub slot_refresh_outcomes: OutcomeCountersSnapshot,
    /// Per-`outcome` split of revoke dispatches (one per *result*).
    pub slot_revoke_outcomes: OutcomeCountersSnapshot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();
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
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();
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
        let m1 = ResourceOpsMetrics::new(&registry).unwrap();
        let m2 = m1.clone();

        m1.record_acquire();
        m2.record_acquire();

        assert_eq!(m1.snapshot().acquire_total, 2);
        assert_eq!(m2.snapshot().acquire_total, 2);
    }

    #[test]
    fn backed_by_registry() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();
        metrics.record_create();
        metrics.record_create();

        // Read directly from registry to verify shared backing.
        let counter = registry.counter(NEBULA_RESOURCE_CREATE_TOTAL).unwrap();
        assert_eq!(counter.get(), 2);
    }

    #[test]
    fn refresh_outcome_split_is_independent_of_attempts() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        // Three dispatches: two ok, one failed. Attempts counts every
        // dispatch; the outcome split partitions by result.
        metrics.record_slot_refresh();
        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Success);
        metrics.record_slot_refresh();
        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Success);
        metrics.record_slot_refresh();
        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Failed);

        let snap = metrics.snapshot();
        assert_eq!(snap.slot_refresh_attempts, 3);
        assert_eq!(snap.slot_refresh_outcomes.success, 2);
        assert_eq!(snap.slot_refresh_outcomes.failed, 1);
        assert_eq!(snap.slot_refresh_outcomes.timed_out, 0);
    }

    #[test]
    fn revoke_outcome_split_counts_timed_out() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        metrics.record_slot_revoke();
        metrics.record_slot_revoke_outcome(SlotDispatchOutcome::Success);
        metrics.record_slot_revoke();
        metrics.record_slot_revoke_outcome(SlotDispatchOutcome::TimedOut);

        let snap = metrics.snapshot();
        assert_eq!(snap.slot_revoke_attempts, 2);
        assert_eq!(snap.slot_revoke_outcomes.success, 1);
        assert_eq!(snap.slot_revoke_outcomes.failed, 0);
        assert_eq!(snap.slot_revoke_outcomes.timed_out, 1);
    }
}
