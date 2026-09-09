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
//! `nebula_metrics::naming::rotation_outcome::{SUCCESS,FAILED,TIMED_OUT,ABANDONED}`.
//! Queue-owned hook settlement records exactly one terminal outcome per
//! admitted dispatch; pre-admission rejection records `failed`. Caller-side
//! observation deferral is not a terminal outcome and is therefore not
//! counted again. It is recorded on the separate observation counter under
//! `rotation_outcome::DEFERRED`. The unlabeled
//! attempts total is the *sum across outcome labels*:
//!
//! ```text
//! attempts == success + failed + timed_out + abandoned
//! ```
//!
//! There is no separate bare attempts counter — that would be a redundant
//! second total of the same events. The labeled counter
//! (`*_ATTEMPTS_TOTAL{outcome=…}`) is the single registry-visible source a
//! scraper observes; [`OutcomeCountersSnapshot`] keeps an in-process view of
//! the same four series for tests and inspection.

use std::time::Duration;

use nebula_metrics::{Counter, Histogram, LabelSet, MetricsRegistry};
use nebula_metrics::{
    MetricsResult,
    naming::{
        NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL, NEBULA_RESOURCE_ACQUIRE_TIMED_OUT_TOTAL,
        NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
        NEBULA_RESOURCE_ACQUIRE_WAITED_TOTAL, NEBULA_RESOURCE_CREATE_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_REVOKE_OBSERVATIONS_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_ROTATION_OBSERVATIONS_TOTAL, NEBULA_RESOURCE_DESTROY_TOTAL,
        NEBULA_RESOURCE_HOLD_DEADLINE_EXCEEDED_TOTAL, NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL,
        NEBULA_RESOURCE_RELEASE_ERROR_TOTAL, NEBULA_RESOURCE_RELEASE_TOTAL, recycle_outcome,
        rotation_outcome,
    },
};

/// Upper bounds (in seconds) for the acquire wait-time histogram's finite
/// buckets — fixed, µs-scale log buckets tuned for acquire waits
/// (tokio-metrics / HikariCP style): `<=100µs`, `<=1ms`, `<=10ms`, `<=100ms`,
/// `<=1s`, `<=10s`, with an implicit final `>10s` overflow bucket
/// ([`Histogram::observe`](nebula_metrics::Histogram::observe) buckets a
/// value into the first boundary `>=` it, so a value exactly on a boundary
/// falls into that bucket, not the next one). A warm pooled/resident hit is
/// low-single-digit microseconds (see `benches/acquire.rs`); these buckets
/// keep that hot path resolvable instead of collapsing into the crate's
/// sub-second-to-10s default histogram layout.
const ACQUIRE_WAIT_BUCKET_BOUNDS_SECONDS: [f64; 6] = [0.0001, 0.001, 0.01, 0.1, 1.0, 10.0];

/// Upper bounds (in microseconds) matching the crate-internal
/// `ACQUIRE_WAIT_BUCKET_BOUNDS_SECONDS`, for callers that want to label
/// [`AcquireWaitSnapshot::buckets`] without re-deriving the table.
/// `bucket_counts[i]` (`i < 6`) counts observations `<= ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS[i]`;
/// `bucket_counts[6]`, one slot past the end of this table, is the `> 10s`
/// overflow bucket — see [`ACQUIRE_WAIT_BUCKET_COUNT`] and
/// [`AcquireWaitSnapshot::buckets`] for the paired accessor that carries this
/// off-by-one relationship in its own return type instead of leaving it to
/// caller bookkeeping.
pub const ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS: [u64; 6] =
    [100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000];

/// Number of acquire-wait histogram buckets, including the implicit `> 10s`
/// overflow bucket — one more than
/// [`ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS`] has entries.
pub const ACQUIRE_WAIT_BUCKET_COUNT: usize = ACQUIRE_WAIT_BUCKET_BOUNDS_SECONDS.len() + 1;

/// Below this, an acquire is "immediate" — pure hot-path overhead, not a
/// real wait. Tuned above the pooled/resident-hit warm-hit baseline
/// (~2µs; see `benches/acquire.rs`) so ordinary jitter on a warm pool does
/// not count as "waited"; anything above this reflects actual contention,
/// creation, or backend work.
const ACQUIRE_IMMEDIATE_THRESHOLD: Duration = Duration::from_micros(50);

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
    slot_refresh_deferred: Counter,
    slot_revoke_deferred: Counter,
    recycle_outcomes: RecycleOutcomeCounters,
    /// Bucketed acquire wait-time distribution. Registered against
    /// [`NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS`] with
    /// [`ACQUIRE_WAIT_BUCKET_BOUNDS_SECONDS`] via
    /// [`MetricsRegistry::histogram_with_buckets_labeled`] and an empty
    /// label set — structurally identical to an unlabeled series
    /// ([`nebula_metrics::MetricKey::unlabeled`] and
    /// `labeled(name, LabelSet::empty())` produce the same key), which is
    /// the only public entry point that accepts custom bucket boundaries.
    /// The histogram's own `_sum`/`_count` exposition fields already carry
    /// everything needed to derive the mean acquire wait, so there is no
    /// separate in-process cumulative-microseconds counter here.
    acquire_wait_seconds: Histogram,
    /// Acquires whose wait exceeded [`ACQUIRE_IMMEDIATE_THRESHOLD`].
    acquire_waited: Counter,
    /// Acquires that failed after the caller's deadline had elapsed.
    acquire_timed_out: Counter,
    /// Hold-deadline watchdog firings (HikariCP `leakDetectionThreshold`
    /// equivalent) — see [`Self::record_hold_deadline_exceeded`].
    hold_deadline_exceeded: Counter,
}

/// How a single per-slot dispatch resolved.
///
/// Closed set mirroring `nebula_metrics::naming::rotation_outcome`. Each
/// dispatch records **exactly one** value; the direction's attempts total is
/// the sum of the four
/// (`attempts == success + failed + timed_out + abandoned`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a slot dispatch outcome must be returned or recorded exactly once"]
#[non_exhaustive]
pub(crate) enum SlotDispatchMetricOutcome {
    /// Hook returned `Ok(())`.
    Success,
    /// Hook returned `Err`.
    Failed,
    /// The admitted hook exceeded its execution timeout.
    TimedOut,
    /// An admitted queue task was abandoned before producing a hook result.
    /// Caller-side receipt deferral does not record this terminal outcome.
    Abandoned,
}

/// Registry-bound `outcome` split for one dispatch direction.
///
/// One physical counter per direction (`*_ATTEMPTS_TOTAL`) carrying the
/// closed `outcome` label set — the four handles below are the
/// `outcome={success,failed,timed_out,abandoned}` series of that one counter, built
/// via [`MetricsRegistry::counter_labeled`] so a scraper observes them.
/// `Clone` is cheap (each [`Counter`] is an `Arc` handle into the shared
/// registry), so clones share the same atomics.
#[derive(Debug, Clone)]
struct OutcomeCounters {
    success: Counter,
    failed: Counter,
    timed_out: Counter,
    abandoned: Counter,
}

impl OutcomeCounters {
    /// Binds the four `outcome`-labeled series of `name` against `registry`.
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
            abandoned: registry
                .counter_labeled(name, &outcome_label(registry, rotation_outcome::ABANDONED))?,
        })
    }

    fn record(&self, outcome: SlotDispatchMetricOutcome) {
        match outcome {
            SlotDispatchMetricOutcome::Success => self.success.inc(),
            SlotDispatchMetricOutcome::Failed => self.failed.inc(),
            SlotDispatchMetricOutcome::TimedOut => self.timed_out.inc(),
            SlotDispatchMetricOutcome::Abandoned => self.abandoned.inc(),
        }
    }

    fn snapshot(&self) -> OutcomeCountersSnapshot {
        OutcomeCountersSnapshot {
            success: self.success.get(),
            failed: self.failed.get(),
            timed_out: self.timed_out.get(),
            abandoned: self.abandoned.get(),
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
    ///
    /// # Errors
    ///
    /// Propagates [`nebula_metrics::MetricsError`] if `registry` rejects a
    /// counter/histogram registration (e.g. a name collision with an
    /// incompatible metric type already registered under the same key).
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
            slot_refresh_deferred: registry.counter_labeled(
                NEBULA_RESOURCE_CREDENTIAL_ROTATION_OBSERVATIONS_TOTAL,
                &outcome_label(registry, rotation_outcome::DEFERRED),
            )?,
            slot_revoke_deferred: registry.counter_labeled(
                NEBULA_RESOURCE_CREDENTIAL_REVOKE_OBSERVATIONS_TOTAL,
                &outcome_label(registry, rotation_outcome::DEFERRED),
            )?,
            recycle_outcomes: RecycleOutcomeCounters::new(registry)?,
            acquire_wait_seconds: registry.histogram_with_buckets_labeled(
                NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
                &LabelSet::empty(),
                ACQUIRE_WAIT_BUCKET_BOUNDS_SECONDS.to_vec(),
            )?,
            acquire_waited: registry.counter(NEBULA_RESOURCE_ACQUIRE_WAITED_TOTAL)?,
            acquire_timed_out: registry.counter(NEBULA_RESOURCE_ACQUIRE_TIMED_OUT_TOTAL)?,
            hold_deadline_exceeded: registry
                .counter(NEBULA_RESOURCE_HOLD_DEADLINE_EXCEEDED_TOTAL)?,
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

    /// Records a release or manager-owned retirement-stage failure.
    ///
    /// Incremented by the bounded release path when `release_one` (a
    /// token return / session close / exclusive reset) — or a follow-up
    /// destroy after a failed reset — returns `Err`. Manager-owned row
    /// retirement also increments once for each failing maintenance or
    /// terminal-cleanup stage. Errors are observed here instead of being
    /// `let _ =`-swallowed.
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
    /// Exactly one terminal outcome is recorded per dispatch, so the
    /// direction's attempts total is
    /// `success + failed + timed_out + abandoned`. Receipt-observation
    /// deferral is not terminal and is not counted. There is no separate bare
    /// attempt counter. `ResourceEvent::SlotRefreshFailed` remains the eventing
    /// surface for failure correlation.
    pub(crate) fn record_slot_refresh_outcome(&self, outcome: SlotDispatchMetricOutcome) {
        self.slot_refresh_outcomes.record(outcome);
    }

    /// Records how one `Manager::revoke_slot` dispatch resolved, bumping the
    /// matching `outcome` series of the revoke attempts counter.
    ///
    /// Same one-outcome-per-dispatch contract as
    /// [`record_slot_refresh_outcome`](Self::record_slot_refresh_outcome):
    /// an admitted task records exactly one of `Success`, `Failed`,
    /// `TimedOut`, or terminal `Abandoned`. A caller that stops
    /// observing an otherwise live queue task records nothing; settlement
    /// later records the task's actual terminal result. Thus
    /// `attempts == success + failed + timed_out + abandoned`.
    pub(crate) fn record_slot_revoke_outcome(&self, outcome: SlotDispatchMetricOutcome) {
        self.slot_revoke_outcomes.record(outcome);
    }

    /// Records that a refresh caller stopped observing an accepted hook.
    /// The queue-owned terminal settlement records separately when it occurs.
    pub(crate) fn record_slot_refresh_deferred(&self) {
        self.slot_refresh_deferred.inc();
    }

    /// Records that a revoke caller stopped observing an accepted hook.
    /// The queue-owned terminal settlement records separately when it occurs.
    pub(crate) fn record_slot_revoke_deferred(&self) {
        self.slot_revoke_deferred.inc();
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

    /// Records one acquire's wait time: buckets it into the acquire
    /// wait-time histogram, and bumps the "did not complete immediately" and
    /// "timed out" sibling counters when applicable.
    ///
    /// Called exactly once per acquire completion (success or failure) from
    /// `Manager::record_acquire_result`, with `elapsed` measured from
    /// acquire entry to guard mint (success) or terminal failure. `timed_out`
    /// is `true` when the caller's [`AcquireOptions`](crate::AcquireOptions)
    /// deadline had already elapsed by the time the (failed) acquire
    /// completed — the caller decides this, since it alone knows the
    /// deadline.
    ///
    /// Hot-path cost: one bucket-index computation (the histogram's internal
    /// binary search), up to three atomic `Relaxed` increments — no locks,
    /// no allocation.
    pub fn record_acquire_wait(&self, elapsed: Duration, timed_out: bool) {
        self.acquire_wait_seconds.observe(elapsed.as_secs_f64());
        if elapsed > ACQUIRE_IMMEDIATE_THRESHOLD {
            self.acquire_waited.inc();
        }
        if timed_out {
            self.acquire_timed_out.inc();
        }
    }

    /// Records one hold-deadline watchdog firing — a lease still held past
    /// its [`Provider::max_hold_duration`](crate::resource::Provider::max_hold_duration)
    /// deadline (HikariCP `leakDetectionThreshold` equivalent). Called by
    /// [`ResourceGuard`](crate::guard::ResourceGuard)'s hold-deadline watchdog
    /// task when it observes the guard still alive past the deadline.
    pub fn record_hold_deadline_exceeded(&self) {
        self.hold_deadline_exceeded.inc();
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
            slot_refresh_deferred: self.slot_refresh_deferred.get(),
            slot_revoke_deferred: self.slot_revoke_deferred.get(),
            recycle_outcomes: self.recycle_outcomes.snapshot(),
            acquire_wait: self.acquire_wait_snapshot(),
            hold_deadline_exceeded: self.hold_deadline_exceeded.get(),
        }
    }

    /// The acquire-wait sub-snapshot: bucket counts read from the
    /// histogram's seqlock-consistent [`nebula_metrics::HistogramSnapshot`],
    /// plus the sibling counters.
    fn acquire_wait_snapshot(&self) -> AcquireWaitSnapshot {
        let histogram = self.acquire_wait_seconds.snapshot();
        let mut bucket_counts = [0u64; ACQUIRE_WAIT_BUCKET_COUNT];
        for (slot, count) in bucket_counts
            .iter_mut()
            .zip(histogram.per_bucket_counts().iter().copied())
        {
            *slot = count;
        }
        AcquireWaitSnapshot {
            waited_count: self.acquire_waited.get(),
            timed_out_count: self.acquire_timed_out.get(),
            bucket_counts,
        }
    }
}

/// Snapshot of the four `outcome`-labeled series of one direction's
/// attempts counter. Mirrors the
/// `nebula_metrics::naming::rotation_outcome` closed label set.
///
/// Queue-owned settlement records exactly one of these per admitted hook;
/// pre-admission rejection records `failed`. The direction's attempts total
/// is the sum:
/// `attempts == success + failed + timed_out + abandoned`.
/// This is an in-process view of the same registry series a scraper reads
/// off `*_ATTEMPTS_TOTAL{outcome=…}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct OutcomeCountersSnapshot {
    /// Resources that completed the dispatch hook with `Ok(())`.
    pub success: u64,
    /// Resources whose hook returned `Err`.
    pub failed: u64,
    /// Resources whose hook exceeded the per-resource timeout budget.
    pub timed_out: u64,
    /// Admitted queue tasks abandoned before a hook result was produced.
    /// This excludes caller-side observation deferral.
    pub abandoned: u64,
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
#[non_exhaustive]
pub struct RecycleOutcomeSnapshot {
    /// Clean leases returned to the idle store.
    pub recycled: u64,
    /// Leases torn down instead of pooled.
    pub discarded: u64,
}

/// Acquire wait-time snapshot: the fixed-bucket histogram distribution plus
/// its sibling counters.
///
/// The bucket counts are non-cumulative — each slot counts only the
/// observations that fell in *that* bucket, not every prior boundary too —
/// and there is one more count than there are entries in
/// [`ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS`]: the last slot is the
/// implicit `> 10s` overflow bucket, which has no finite upper bound to
/// pair with. Use [`buckets`](Self::buckets) to iterate counts paired with
/// their bound (`None` for the overflow slot) without re-deriving this
/// off-by-one relationship at the call site, or
/// [`bucket_counts`](Self::bucket_counts) for the raw counts. `waited_count`
/// and `timed_out_count` are the two derived yes/no signals HikariCP-style
/// dashboards want without computing them from the buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct AcquireWaitSnapshot {
    /// Acquires whose wait exceeded the crate's "did not complete
    /// immediately" threshold (see `ACQUIRE_IMMEDIATE_THRESHOLD`).
    pub waited_count: u64,
    /// Acquires that failed after the caller's `AcquireOptions` deadline had
    /// already elapsed.
    pub timed_out_count: u64,
    /// Non-cumulative per-bucket observation counts; see the struct docs for
    /// the indexing contract. Private so a future bucket-count change stays
    /// additive — read via [`bucket_counts`](Self::bucket_counts) or
    /// [`buckets`](Self::buckets).
    bucket_counts: [u64; ACQUIRE_WAIT_BUCKET_COUNT],
}

impl AcquireWaitSnapshot {
    /// Raw non-cumulative per-bucket observation counts, one longer than
    /// [`ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS`] (the trailing count is the
    /// `> 10s` overflow bucket). Prefer [`buckets`](Self::buckets) when you
    /// want each count paired with its upper bound instead of re-deriving
    /// the indexing relationship yourself.
    #[must_use]
    pub fn bucket_counts(&self) -> &[u64] {
        &self.bucket_counts
    }

    /// Iterates the per-bucket counts paired with their upper bound in
    /// microseconds: `Some(bound)` for each of the
    /// [`ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS`] finite buckets, then
    /// `None` for the trailing `> 10s` overflow bucket. Carries the
    /// counts-vs-bounds off-by-one in its own return type so callers never
    /// have to reason about it, and stays correct if a future bucket is
    /// added to both tables.
    pub fn buckets(&self) -> impl Iterator<Item = (Option<u64>, u64)> + '_ {
        self.bucket_counts.iter().enumerate().map(|(i, &count)| {
            (
                ACQUIRE_WAIT_BUCKET_UPPER_BOUNDS_MICROS.get(i).copied(),
                count,
            )
        })
    }
}

/// Point-in-time snapshot of resource operation counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
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
    /// `success + failed + timed_out + abandoned`; see [`OutcomeCountersSnapshot`].
    pub slot_refresh_outcomes: OutcomeCountersSnapshot,
    /// Per-`outcome` split of revoke dispatches (`Manager::revoke_slot`),
    /// one increment per dispatch. The revoke attempts total is
    /// `success + failed + timed_out + abandoned`.
    pub slot_revoke_outcomes: OutcomeCountersSnapshot,
    /// Accepted refresh hooks whose callers stopped waiting while the work
    /// remained queue-owned and in flight.
    pub slot_refresh_deferred: u64,
    /// Accepted revoke hooks whose callers stopped waiting while the work
    /// remained queue-owned and in flight.
    pub slot_revoke_deferred: u64,
    /// Per-`outcome` split of pooled releases, one increment per release.
    /// The release total is `recycled + discarded`; see
    /// [`RecycleOutcomeSnapshot`]. A pool with `recycled == 0` and
    /// `discarded > 0` is silently discarding every instance.
    pub recycle_outcomes: RecycleOutcomeSnapshot,
    /// Acquire wait-time histogram + waited/timed-out counters. See
    /// [`AcquireWaitSnapshot`].
    pub acquire_wait: AcquireWaitSnapshot,
    /// Hold-deadline watchdog firings (HikariCP `leakDetectionThreshold`
    /// equivalent) — a lease still held past
    /// [`Provider::max_hold_duration`](crate::resource::Provider::max_hold_duration)
    /// when the watchdog checked. Warn-only; the lease is not forcibly
    /// released. A non-zero, climbing count is a leaked-guard or
    /// stuck-caller signal.
    pub hold_deadline_exceeded: u64,
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

        // Four dispatches: two completed, one failed, one abandoned. One
        // outcome per dispatch, so attempts is the sum of all four labels.
        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Abandoned);

        let snap = metrics.snapshot();
        let o = snap.slot_refresh_outcomes;
        assert_eq!(o.success, 2);
        assert_eq!(o.failed, 1);
        assert_eq!(o.timed_out, 0);
        assert_eq!(o.abandoned, 1);
        assert_eq!(
            o.success + o.failed + o.timed_out + o.abandoned,
            4,
            "attempts == Σ outcomes"
        );
    }

    #[test]
    fn revoke_outcome_split_counts_timed_out() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Success);
        metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::TimedOut);
        metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Abandoned);

        let snap = metrics.snapshot();
        let o = snap.slot_revoke_outcomes;
        assert_eq!(o.success, 1);
        assert_eq!(o.failed, 0);
        assert_eq!(o.timed_out, 1);
        assert_eq!(o.abandoned, 1);
    }

    #[test]
    fn deferred_observation_is_separate_from_terminal_attempts() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        metrics.record_slot_refresh_deferred();
        metrics.record_slot_revoke_deferred();
        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
        metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::Abandoned);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.slot_refresh_deferred, 1);
        assert_eq!(snapshot.slot_revoke_deferred, 1);
        assert_eq!(snapshot.slot_refresh_outcomes.success, 1);
        assert_eq!(snapshot.slot_refresh_outcomes.abandoned, 0);
        assert_eq!(snapshot.slot_revoke_outcomes.success, 0);
        assert_eq!(snapshot.slot_revoke_outcomes.abandoned, 1);
    }

    /// The per-`outcome` split must reach the shared registry — the same
    /// `(name, outcome=<value>)` series the manager wrote is observable
    /// through a sibling `counter_labeled` handle and is enumerated by
    /// `snapshot_counters` (what an exporter scrapes).
    #[test]
    fn outcome_split_is_registry_bound() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Success);
        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Failed);
        metrics.record_slot_refresh_outcome(SlotDispatchMetricOutcome::Abandoned);
        metrics.record_slot_revoke_outcome(SlotDispatchMetricOutcome::TimedOut);

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

        let refresh_abandoned = registry
            .counter_labeled(
                NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
                &outcome_label(&registry, rotation_outcome::ABANDONED),
            )
            .unwrap();
        assert_eq!(
            refresh_abandoned.get(),
            1,
            "refresh abandonment must be registry-bound"
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
            labeled_series, 4,
            "all four outcome series of the refresh attempts counter must be registered"
        );
    }

    // ── acquire wait-time histogram + waited/timed-out counters ────────────

    #[test]
    fn acquire_wait_starts_at_zero() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();
        let snap = metrics.snapshot().acquire_wait;
        assert_eq!(snap.waited_count, 0);
        assert_eq!(snap.timed_out_count, 0);
        assert_eq!(
            snap.bucket_counts(),
            [0u64; ACQUIRE_WAIT_BUCKET_COUNT].as_slice()
        );
    }

    #[test]
    fn record_acquire_wait_falls_in_the_expected_bucket() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        // 5µs: well under the 100µs first bucket — the hot pooled/resident-hit
        // path (~2µs warm-hit baseline; see benches/acquire.rs) must resolve
        // here, not collapse into a coarser bucket.
        metrics.record_acquire_wait(Duration::from_micros(5), false);
        // 50ms: falls in the <=100ms bucket (index 3).
        metrics.record_acquire_wait(Duration::from_millis(50), false);
        // 20s: exceeds every finite bound — the `> 10s` overflow bucket.
        metrics.record_acquire_wait(Duration::from_secs(20), false);

        let snap = metrics.snapshot().acquire_wait;
        assert_eq!(
            snap.bucket_counts()[0],
            1,
            "5µs must land in the <=100µs bucket"
        );
        assert_eq!(
            snap.bucket_counts()[3],
            1,
            "50ms must land in the <=100ms bucket"
        );
        assert_eq!(
            snap.bucket_counts()[ACQUIRE_WAIT_BUCKET_COUNT - 1],
            1,
            "20s must land in the >10s overflow bucket"
        );
        assert_eq!(
            snap.bucket_counts().iter().sum::<u64>(),
            3,
            "every observation must land in exactly one bucket"
        );
        assert_eq!(
            snap.buckets().last(),
            Some((None, 1)),
            "the last paired entry is the overflow bucket, with no finite upper bound"
        );
    }

    #[test]
    fn record_acquire_wait_tracks_waited() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        // Below the immediate threshold: not "waited".
        metrics.record_acquire_wait(Duration::from_micros(5), false);
        // Above it: counts as "waited".
        metrics.record_acquire_wait(Duration::from_millis(1), false);

        let snap = metrics.snapshot().acquire_wait;
        assert_eq!(
            snap.waited_count, 1,
            "only the above-threshold acquire counts as waited"
        );
    }

    #[test]
    fn record_acquire_wait_counts_timed_out_only_when_flagged() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();

        metrics.record_acquire_wait(Duration::from_millis(1), false);
        metrics.record_acquire_wait(Duration::from_millis(1), true);

        let snap = metrics.snapshot().acquire_wait;
        assert_eq!(snap.timed_out_count, 1);
    }

    #[test]
    fn acquire_wait_histogram_is_registry_bound() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();
        metrics.record_acquire_wait(Duration::from_millis(1), false);

        // A sibling handle built the same way (same name, empty label set,
        // same custom buckets) must see the same underlying series — proving
        // the empty-`LabelSet` construction in `ResourceOpsMetrics::new` is
        // registry-bound and not a private, unexported histogram.
        let sibling = registry
            .histogram_with_buckets_labeled(
                NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
                &LabelSet::empty(),
                ACQUIRE_WAIT_BUCKET_BOUNDS_SECONDS.to_vec(),
            )
            .unwrap();
        assert_eq!(
            sibling.count(),
            1,
            "acquire-wait histogram must be registry-bound"
        );

        // And an unlabeled lookup resolves to the very same `MetricKey`
        // (empty label set == unlabeled) — this is *why* the custom-bucket
        // empty-labeled construction works as the crate's de facto unlabeled
        // custom-bucket entry point.
        let name_spur = registry
            .interner()
            .intern(NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS);
        let unlabeled_series = registry
            .snapshot_histograms()
            .into_iter()
            .filter(|(k, _)| k.name == name_spur && k.labels.is_empty())
            .count();
        assert_eq!(
            unlabeled_series, 1,
            "the acquire-wait histogram must be the sole unlabeled series under its name"
        );
    }

    // ── hold-deadline-exceeded counter ──────────────────────────────────────

    #[test]
    fn record_hold_deadline_exceeded_increments() {
        let registry = MetricsRegistry::new();
        let metrics = ResourceOpsMetrics::new(&registry).unwrap();
        assert_eq!(metrics.snapshot().hold_deadline_exceeded, 0);
        metrics.record_hold_deadline_exceeded();
        metrics.record_hold_deadline_exceeded();
        assert_eq!(metrics.snapshot().hold_deadline_exceeded, 2);
    }
}
