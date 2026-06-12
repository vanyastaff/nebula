//! Registry-backed counters for resource operation tracking.
//!
//! [`ResourceOpsMetrics`] wraps a fixed set of [`Counter`]s from a shared
//! [`MetricsRegistry`], replacing the previous hand-rolled atomic
//! counters. Use [`snapshot()`](ResourceOpsMetrics::snapshot) to capture a
//! point-in-time view as [`ResourceOpsSnapshot`].
//!
//! Per slot model the credential rotation/revoke counters that the previous
//! `Resource::Credential` model owned have been removed. The per-slot
//! rotation hook shape is `on_credential_refresh(&self, slot_name, runtime)`
//! / `on_credential_revoke(&self, slot_name, runtime)`.
//!
//! Per credential isolation the rotation/revoke attempt totals are **one counter per
//! direction, labeled by `outcome`** over the closed set
//! `nebula_metrics::naming::rotation_outcome::{SUCCESS,FAILED,TIMED_OUT}`.
//! `Manager::{refresh_slot,revoke_slot}` record exactly one outcome per
//! dispatch from their `Ok` / `Err` / drain-timeout arms, so the unlabeled
//! attempts total is the *sum across outcome labels*:
//!
//! ```text
//! attempts == success + failed + timed_out
//! ```
//!
//! There is no separate bare attempts counter — that would be a redundant
//! second total of the same events. The labeled counter
//! (`*_ATTEMPTS_TOTAL{outcome=…}`) is the single registry-visible source a
//! scraper observes; [`OutcomeCountersSnapshot`] keeps an in-process view of
//! the same three series for tests and inspection.

use nebula_metrics::{Counter, LabelSet, MetricsRegistry};
use nebula_metrics::{
    MetricsResult,
    naming::{
        NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL, NEBULA_RESOURCE_ACQUIRE_TOTAL,
        NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL, NEBULA_RESOURCE_DESTROY_TOTAL,
        NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL, NEBULA_RESOURCE_RELEASE_ERROR_TOTAL,
        NEBULA_RESOURCE_RELEASE_TOTAL, recycle_outcome, rotation_outcome,
    },
};

/// Builds the `outcome=<value>` label set for the rotation/revoke attempt
/// counters, interned against the registry that owns the series.
///
/// Mirrors the `reclaim_label` helper in
/// `engine::credential::refresh::metrics`: the `outcome` key is the closed
/// dimension declared alongside
/// [`NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL`].
fn outcome_label(registry: &MetricsRegistry, outcome: &str) -> LabelSet {
    registry.interner().single("outcome", outcome)
}

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
/// metrics.record_release_error();
///
/// let snap = metrics.snapshot();
/// assert_eq!(snap.acquire_total, 2);
/// assert_eq!(snap.acquire_errors, 1);
/// assert_eq!(snap.release_errors, 1);
/// ```
#[derive(Debug, Clone)]
pub struct ResourceOpsMetrics {
    acquire_total: Counter,
    acquire_errors: Counter,
    release_total: Counter,
    release_errors: Counter,
    create_total: Counter,
    destroy_total: Counter,
    slot_refresh_outcomes: OutcomeCounters,
    slot_revoke_outcomes: OutcomeCounters,
    recycle_outcomes: RecycleOutcomeCounters,
}

/// How a single per-slot dispatch resolved.
///
/// Closed set mirroring `nebula_metrics::naming::rotation_outcome`. Each
/// dispatch records **exactly one** value; the direction's attempts total is
/// the sum of the three (`attempts == success + failed + timed_out`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotDispatchOutcome {
    /// Hook returned `Ok(())`.
    Success,
    /// Hook returned `Err`.
    Failed,
    /// Bounded in-flight drain elapsed before the hook ran (`revoke_slot`).
    TimedOut,
}

/// Registry-bound `outcome` split for one dispatch direction.
///
/// One physical counter per direction (`*_ATTEMPTS_TOTAL`) carrying the
/// closed `outcome` label set — the three handles below are the
/// `outcome={success,failed,timed_out}` series of that one counter, built
/// via [`MetricsRegistry::counter_labeled`] so a scraper observes them.
/// `Clone` is cheap (each [`Counter`] is an `Arc` handle into the shared
/// registry), so clones share the same atomics.
#[derive(Debug, Clone)]
struct OutcomeCounters {
    success: Counter,
    failed: Counter,
    timed_out: Counter,
}

impl OutcomeCounters {
    /// Binds the three `outcome`-labeled series of `name` against `registry`.
    ///
    /// `name` is the direction's attempts constant
    /// ([`NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL`] for refresh,
    /// [`NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL`] for revoke).
    fn new(registry: &MetricsRegistry, name: &str) -> MetricsResult<Self> {
        Ok(Self {
            success: registry
                .counter_labeled(name, &outcome_label(registry, rotation_outcome::SUCCESS))?,
            failed: registry
                .counter_labeled(name, &outcome_label(registry, rotation_outcome::FAILED))?,
            timed_out: registry
                .counter_labeled(name, &outcome_label(registry, rotation_outcome::TIMED_OUT))?,
        })
    }

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

/// How a single pooled-release resolved on the framework release path.
///
/// Closed two-way split mirroring `nebula_metrics::naming::recycle_outcome`.
/// Every `release_slot` call records **exactly one** value: `Recycled` when
/// the clean lease is returned to the idle store, `Discarded` on every
/// teardown path (tainted, reset error, evict-on-return, non-pooling / `Drop`
/// decision). The release total is `recycled + discarded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecycleOutcome {
    /// Clean lease returned to the idle store and reusable.
    Recycled,
    /// Lease torn down instead of pooled.
    Discarded,
}

/// Registry-bound `outcome` split for the pooled-release recycle counter.
///
/// One physical counter ([`NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL`]) carrying
/// the closed `outcome={recycled,discarded}` label set; the two handles
/// below are those two series, built via
/// [`MetricsRegistry::counter_labeled`] so a scraper observes them. `Clone`
/// is cheap (each [`Counter`] is an `Arc` handle into the shared registry),
/// so clones share the same atomics.
#[derive(Debug, Clone)]
struct RecycleOutcomeCounters {
    recycled: Counter,
    discarded: Counter,
}

impl RecycleOutcomeCounters {
    /// Binds the two `outcome`-labeled series of
    /// [`NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL`] against `registry`.
    fn new(registry: &MetricsRegistry) -> MetricsResult<Self> {
        Ok(Self {
            recycled: registry.counter_labeled(
                NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL,
                &outcome_label(registry, recycle_outcome::RECYCLED),
            )?,
            discarded: registry.counter_labeled(
                NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL,
                &outcome_label(registry, recycle_outcome::DISCARDED),
            )?,
        })
    }

    fn record(&self, outcome: RecycleOutcome) {
        match outcome {
            RecycleOutcome::Recycled => self.recycled.inc(),
            RecycleOutcome::Discarded => self.discarded.inc(),
        }
    }

    fn snapshot(&self) -> RecycleOutcomeSnapshot {
        RecycleOutcomeSnapshot {
            recycled: self.recycled.get(),
            discarded: self.discarded.get(),
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
            release_errors: registry.counter(NEBULA_RESOURCE_RELEASE_ERROR_TOTAL)?,
            create_total: registry.counter(NEBULA_RESOURCE_CREATE_TOTAL)?,
            destroy_total: registry.counter(NEBULA_RESOURCE_DESTROY_TOTAL)?,
            slot_refresh_outcomes: OutcomeCounters::new(
                registry,
                NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
            )?,
            slot_revoke_outcomes: OutcomeCounters::new(
                registry,
                NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
            )?,
            recycle_outcomes: RecycleOutcomeCounters::new(registry)?,
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

    /// Records a release-hook failure.
    ///
    /// Incremented by the bounded release path when `release_one` (a
    /// token return / session close / exclusive reset) — or a follow-up
    /// destroy after a failed reset — returns `Err`. The error is observed
    /// here instead of being `let _ =`-swallowed (R17).
    pub fn record_release_error(&self) {
        self.release_errors.inc();
    }

    /// Records a new resource instance creation.
    pub fn record_create(&self) {
        self.create_total.inc();
    }

    /// Records a resource instance destruction.
    pub fn record_destroy(&self) {
        self.destroy_total.inc();
    }

    /// Records how one `Manager::refresh_slot` dispatch resolved, bumping the
    /// matching `outcome` series of the refresh attempts counter.
    ///
    /// Exactly one outcome is recorded per dispatch, so the direction's
    /// attempts total is `success + failed + timed_out`. There is no separate
    /// bare attempt counter. `ResourceEvent::SlotRefreshFailed` remains the
    /// eventing surface for failure correlation.
    pub fn record_slot_refresh_outcome(&self, outcome: SlotDispatchOutcome) {
        self.slot_refresh_outcomes.record(outcome);
    }

    /// Records how one `Manager::revoke_slot` dispatch resolved, bumping the
    /// matching `outcome` series of the revoke attempts counter.
    ///
    /// Same one-outcome-per-dispatch contract as
    /// [`record_slot_refresh_outcome`](Self::record_slot_refresh_outcome):
    /// the bounded in-flight drain expiring records `TimedOut` and is
    /// *terminal* for that dispatch (no subsequent `Success`/`Failed` for
    /// the same revoke), so `attempts == success + failed + timed_out`.
    pub fn record_slot_revoke_outcome(&self, outcome: SlotDispatchOutcome) {
        self.slot_revoke_outcomes.record(outcome);
    }

    /// Records how one pooled release resolved, bumping the matching
    /// `outcome` series of the recycle counter.
    ///
    /// Called exactly once per framework release (`release_slot`):
    /// [`Recycled`](RecycleOutcome::Recycled) when the clean lease is returned
    /// to the idle store, [`Discarded`](RecycleOutcome::Discarded) on every
    /// teardown path (tainted lease, reset error, evict-on-return, or a
    /// non-pooling / `Drop` recycle decision). One outcome per release, so the
    /// release total is `recycled + discarded`; a pool stuck at
    /// `discarded == releases` with zero recycles is a silently-evicting pool.
    pub fn record_recycle_outcome(&self, outcome: RecycleOutcome) {
        self.recycle_outcomes.record(outcome);
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
            release_errors: self.release_errors.get(),
            create_total: self.create_total.get(),
            destroy_total: self.destroy_total.get(),
            slot_refresh_outcomes: self.slot_refresh_outcomes.snapshot(),
            slot_revoke_outcomes: self.slot_revoke_outcomes.snapshot(),
            recycle_outcomes: self.recycle_outcomes.snapshot(),
        }
    }
}

/// Snapshot of the three `outcome`-labeled series of one direction's
/// attempts counter. Mirrors the
/// `nebula_metrics::naming::rotation_outcome` closed label set.
///
/// `Manager::{refresh_slot,revoke_slot}` record exactly one of these per
/// dispatch from their `Ok` / `Err` / drain-timeout arms, so the direction's
/// attempts total is the sum: `attempts == success + failed + timed_out`.
/// This is an in-process view of the same registry series a scraper reads
/// off `*_ATTEMPTS_TOTAL{outcome=…}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutcomeCountersSnapshot {
    /// Resources that completed the dispatch hook with `Ok(())`.
    pub success: u64,
    /// Resources whose hook returned `Err`.
    pub failed: u64,
    /// Resources whose hook exceeded the per-resource timeout budget.
    pub timed_out: u64,
}

/// Snapshot of the two `outcome`-labeled series of the pooled-release recycle
/// counter. Mirrors the `nebula_metrics::naming::recycle_outcome` closed
/// label set.
///
/// `release_slot` records exactly one of these per release, so the release
/// total is `recycled + discarded`. This is an in-process view of the same
/// registry series a scraper reads off the
/// `NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL` `outcome` label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecycleOutcomeSnapshot {
    /// Clean leases returned to the idle store.
    pub recycled: u64,
    /// Leases torn down instead of pooled.
    pub discarded: u64,
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
    /// Total release-hook failures (`release_one` / reset / close /
    /// post-failed-reset destroy returned `Err`). Observed, not swallowed.
    pub release_errors: u64,
    /// Total resource instances created.
    pub create_total: u64,
    /// Total resource instances destroyed.
    pub destroy_total: u64,
    /// Per-`outcome` split of refresh dispatches (`Manager::refresh_slot`),
    /// one increment per dispatch. The refresh attempts total is
    /// `success + failed + timed_out`; see [`OutcomeCountersSnapshot`].
    pub slot_refresh_outcomes: OutcomeCountersSnapshot,
    /// Per-`outcome` split of revoke dispatches (`Manager::revoke_slot`),
    /// one increment per dispatch. The revoke attempts total is
    /// `success + failed + timed_out`.
    pub slot_revoke_outcomes: OutcomeCountersSnapshot,
    /// Per-`outcome` split of pooled releases, one increment per release.
    /// The release total is `recycled + discarded`; see
    /// [`RecycleOutcomeSnapshot`]. A pool with `recycled == 0` and
    /// `discarded > 0` is silently discarding every instance.
    pub recycle_outcomes: RecycleOutcomeSnapshot,
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
        assert_eq!(snap.release_errors, 0);
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
        metrics.record_release_error();
        metrics.record_release_error();
        metrics.record_create();
        metrics.record_create();
        metrics.record_create();
        metrics.record_destroy();

        let snap = metrics.snapshot();
        assert_eq!(snap.acquire_total, 2);
        assert_eq!(snap.acquire_errors, 1);
        assert_eq!(snap.release_total, 1);
        assert_eq!(snap.release_errors, 2);
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
    fn refresh_attempts_is_sum_of_outcomes() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        // Three dispatches: two ok, one failed. One outcome per dispatch,
        // so attempts == success + failed + timed_out.
        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Success);
        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Success);
        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Failed);

        let snap = metrics.snapshot();
        let o = snap.slot_refresh_outcomes;
        assert_eq!(o.success, 2);
        assert_eq!(o.failed, 1);
        assert_eq!(o.timed_out, 0);
        assert_eq!(
            o.success + o.failed + o.timed_out,
            3,
            "attempts == Σ outcomes"
        );
    }

    #[test]
    fn revoke_outcome_split_counts_timed_out() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        metrics.record_slot_revoke_outcome(SlotDispatchOutcome::Success);
        metrics.record_slot_revoke_outcome(SlotDispatchOutcome::TimedOut);

        let snap = metrics.snapshot();
        let o = snap.slot_revoke_outcomes;
        assert_eq!(o.success, 1);
        assert_eq!(o.failed, 0);
        assert_eq!(o.timed_out, 1);
    }

    /// The per-`outcome` split must reach the shared registry — the same
    /// `(name, outcome=<value>)` series the manager wrote is observable
    /// through a sibling `counter_labeled` handle and is enumerated by
    /// `snapshot_counters` (what an exporter scrapes).
    #[test]
    fn outcome_split_is_registry_bound() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Success);
        metrics.record_slot_refresh_outcome(SlotDispatchOutcome::Failed);
        metrics.record_slot_revoke_outcome(SlotDispatchOutcome::TimedOut);

        // Sibling handle on the same registry sees the same atomic.
        let refresh_success = registry
            .counter_labeled(
                NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
                &outcome_label(&registry, rotation_outcome::SUCCESS),
            )
            .unwrap();
        assert_eq!(
            refresh_success.get(),
            1,
            "refresh success must be registry-bound"
        );

        let revoke_timed_out = registry
            .counter_labeled(
                NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
                &outcome_label(&registry, rotation_outcome::TIMED_OUT),
            )
            .unwrap();
        assert_eq!(
            revoke_timed_out.get(),
            1,
            "revoke timed_out must be registry-bound"
        );

        // And the series is enumerated by the exporter-facing snapshot.
        let name_spur = registry
            .interner()
            .intern(NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL);
        let labeled_series = registry
            .snapshot_counters()
            .into_iter()
            .filter(|(k, _)| k.name == name_spur && !k.labels.is_empty())
            .count();
        assert_eq!(
            labeled_series, 3,
            "all three outcome series of the refresh attempts counter must be registered"
        );
    }
}
