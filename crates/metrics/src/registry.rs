//! Metrics primitives and registry.
//!
//! Provides lightweight metric types (counter, gauge, histogram) and a
//! registry to create and retrieve them. The implementation stores values
//! in-memory with atomics and uses [`lasso`]-backed string interning plus
//! [`dashmap`]-backed sharded maps for low-latency concurrent access on the
//! hot recording path.

use std::{
    hint::spin_loop,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering, fence},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::{DashMap, mapref::entry::Entry};
use lasso::Spur;

use crate::{
    error::{MetricKind, MetricsError, MetricsResult},
    labels::{LabelInterner, LabelSet, MetricKey},
};

/// Returns the current time as milliseconds since the Unix epoch.
///
/// Wall-clock steps backward ([`SystemTime::duration_since`] failure) map
/// to zero duration, so timestamps can move backward and interact poorly with
/// [`MetricsRegistry::retain_recent`]. Callers should not rely on strict
/// monotonicity of this clock.
#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// An incrementing counter.
#[derive(Debug, Clone)]
pub struct Counter {
    value: Arc<AtomicU64>,
    last_updated_ms: Arc<AtomicU64>,
}

impl Counter {
    /// Create a new counter starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicU64::new(0)),
            last_updated_ms: Arc::new(AtomicU64::new(now_ms())),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Increment by a given amount.
    ///
    /// `inc_by(0)` does not change the stored value or
    /// [`Self::last_updated_ms`] (see [`nebula_metrics::MetricsRegistry::retain_recent`]).
    pub fn inc_by(&self, n: u64) {
        if n == 0 {
            return;
        }
        self.value.fetch_add(n, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Milliseconds since Unix epoch of the last write to this counter.
    #[must_use]
    pub fn last_updated_ms(&self) -> u64 {
        self.last_updated_ms.load(Ordering::Relaxed)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// A gauge that can go up and down.
#[derive(Debug, Clone)]
pub struct Gauge {
    value: Arc<AtomicI64>,
    last_updated_ms: Arc<AtomicU64>,
}

impl Gauge {
    /// Create a new gauge starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicI64::new(0)),
            last_updated_ms: Arc::new(AtomicU64::new(now_ms())),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Decrement by one.
    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Set to a specific value.
    ///
    /// If `v` equals the value already stored, [`Self::last_updated_ms`] is
    /// left unchanged so idle gauges are not kept artificially "fresh" for
    /// retention heuristics.
    pub fn set(&self, v: i64) {
        let previous = self.value.swap(v, Ordering::Relaxed);
        if previous != v {
            self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
        }
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Milliseconds since Unix epoch of the last write to this gauge.
    #[must_use]
    pub fn last_updated_ms(&self) -> u64 {
        self.last_updated_ms.load(Ordering::Relaxed)
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

/// Default finite upper bounds for the built-in histogram layout (sub-second
/// through 10 seconds, suited to latency-style measurements in seconds).
const DEFAULT_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Frozen, point-in-time view of histogram bucket counts, total count, and sum.
///
/// Produced by [`Histogram::snapshot`] using a seqlock (sequentially consistent
/// phase counter + fences) so count, sum, and per-bucket tallies correspond to
/// **one** logical state without blocking [`Histogram::observe`].
///
/// This type does **not** pin future observations; only the numeric fields
/// inside this value are immutable.
#[derive(Debug, Clone)]
pub struct HistogramSnapshot {
    boundaries: Arc<Vec<f64>>,
    per_bucket: Box<[u64]>,
    observation_count: u64,
    sum_value: f64,
}

impl HistogramSnapshot {
    /// Total number of observations reflected in this snapshot.
    #[must_use]
    pub fn observation_count(&self) -> u64 {
        self.observation_count
    }

    /// Sum of observations reflected in this snapshot.
    #[must_use]
    pub fn sum(&self) -> f64 {
        self.sum_value
    }

    /// Upper bounds for finite buckets (excludes the implicit `+Inf` bucket).
    #[must_use]
    pub fn boundaries(&self) -> &[f64] {
        self.boundaries.as_slice()
    }

    /// Non-cumulative observation count per histogram bucket (`+Inf` is the final slot).
    #[must_use]
    pub fn per_bucket_counts(&self) -> &[u64] {
        &self.per_bucket
    }

    /// Cumulative `(upper_bound, cumulative_count)` pairs, including `+Inf` as the final upper
    /// bound.
    #[must_use]
    pub fn cumulative_buckets(&self) -> Vec<(f64, u64)> {
        let mut cumulative = 0u64;
        let mut result = Vec::with_capacity(self.per_bucket.len());
        for (i, count) in self.per_bucket.iter().enumerate() {
            cumulative += *count;
            let upper = if i < self.boundaries.len() {
                self.boundaries[i]
            } else {
                f64::INFINITY
            };
            result.push((upper, cumulative));
        }
        result
    }
}

/// A histogram that records observations into fixed buckets.
///
/// Uses a fixed set of finite upper bounds plus an implicit `+Inf` overflow
/// bucket. Each observation increments the appropriate bucket counter
/// atomically. Recording never takes a mutex: concurrent [`Self::observe`] uses
/// a seqlock bracket; scrapers retry [`Self::snapshot`] briefly while writers
/// race. Hot-path updates remain lock-free besides two `SeqCst` bumps per
/// observation. Sum uses [`AtomicU64::update`] on `f64` bits.
///
/// Prefer [`Histogram::try_with_buckets`] for caller-supplied boundaries; the
/// default layout from [`Histogram::new`] uses the crate's built-in boundary
/// table (see `default_bucket_table_is_valid`).
#[derive(Debug)]
pub struct Histogram {
    /// Odd while an observation is committing; bumped with `Ordering::SeqCst`.
    seq: Arc<AtomicU64>,
    /// Upper-bound for each bucket (sorted, does not include +Inf).
    boundaries: Arc<Vec<f64>>,
    /// Non-cumulative count per bucket (`len == boundaries.len() + 1` for `+Inf`).
    counts: Arc<Vec<AtomicU64>>,
    /// Total number of observations.
    total_count: Arc<AtomicU64>,
    /// Sum of all observed values (stored as f64 bits).
    sum_bits: Arc<AtomicU64>,
    /// Milliseconds since Unix epoch of the last observation.
    last_updated_ms: Arc<AtomicU64>,
}

impl Clone for Histogram {
    fn clone(&self) -> Self {
        Self {
            seq: Arc::clone(&self.seq),
            boundaries: Arc::clone(&self.boundaries),
            counts: Arc::clone(&self.counts),
            total_count: Arc::clone(&self.total_count),
            sum_bits: Arc::clone(&self.sum_bits),
            last_updated_ms: Arc::clone(&self.last_updated_ms),
        }
    }
}

impl Histogram {
    /// Validate histogram bucket boundaries for [`Self::try_with_buckets`].
    pub fn validate_bucket_boundaries(boundaries: &[f64]) -> MetricsResult<()> {
        if boundaries.is_empty() {
            return Err(MetricsError::InvalidHistogramBuckets {
                reason: "boundaries must not be empty".into(),
            });
        }
        if !boundaries.iter().all(|&b| b > 0.0 && b.is_finite()) {
            return Err(MetricsError::InvalidHistogramBuckets {
                reason: "each boundary must be positive and finite".into(),
            });
        }
        if !boundaries.windows(2).all(|w| w[0] < w[1]) {
            return Err(MetricsError::InvalidHistogramBuckets {
                reason: "boundaries must be strictly increasing with no duplicates".into(),
            });
        }
        Ok(())
    }

    fn from_validated_boundaries(boundaries: Vec<f64>) -> Self {
        let bucket_count = boundaries.len() + 1; // +1 for +Inf
        let counts: Vec<AtomicU64> = (0..bucket_count).map(|_| AtomicU64::new(0)).collect();

        tracing::debug!(buckets = boundaries.len(), "histogram created");

        Self {
            seq: Arc::new(AtomicU64::new(0)),
            boundaries: Arc::new(boundaries),
            counts: Arc::new(counts),
            total_count: Arc::new(AtomicU64::new(0)),
            sum_bits: Arc::new(AtomicU64::new(0.0_f64.to_bits())),
            last_updated_ms: Arc::new(AtomicU64::new(now_ms())),
        }
    }

    /// Create a histogram with the built-in default bucket layout.
    ///
    /// The default boundary table is fixed at compile time; if it were ever
    /// invalid, this would construct an inconsistent histogram — covered by
    /// `default_bucket_table_is_valid`.
    #[must_use]
    pub fn new() -> Self {
        Self::from_validated_boundaries(DEFAULT_BUCKETS.to_vec())
    }

    /// Create a histogram with custom bucket boundaries.
    pub fn try_with_buckets(boundaries: Vec<f64>) -> MetricsResult<Self> {
        Self::validate_bucket_boundaries(&boundaries)?;
        Ok(Self::from_validated_boundaries(boundaries))
    }

    /// Record an observation.
    ///
    /// Non-finite values (`NaN`, `±∞`) are silently dropped. NaN would
    /// otherwise permanently poison `sum_bits` via the atomic update
    /// (`x + NaN = NaN`), breaking every subsequent `sum()` / percentile.
    pub fn observe(&self, value: f64) {
        if !value.is_finite() {
            return;
        }

        self.seq.fetch_add(1, Ordering::SeqCst);

        // Find the first bucket whose upper bound is >= value.
        let idx = self
            .boundaries
            .binary_search_by(|bound| {
                bound
                    .partial_cmp(&value)
                    .unwrap_or(std::cmp::Ordering::Less)
            })
            .unwrap_or_else(|insert_pos| insert_pos);

        self.counts[idx].fetch_add(1, Ordering::Relaxed);
        self.total_count.fetch_add(1, Ordering::Relaxed);

        // Atomically add to sum using `AtomicU64::update` on f64 bits (Rust 1.95).
        // Load and store orderings both Relaxed — match the prior CAS loop.
        let _ = self
            .sum_bits
            .update(Ordering::Relaxed, Ordering::Relaxed, |old_bits| {
                (f64::from_bits(old_bits) + value).to_bits()
            });
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);

        fence(Ordering::SeqCst);
        self.seq.fetch_add(1, Ordering::SeqCst);
    }

    /// Capture counts, observation total, and sum at one logical instant.
    ///
    /// Intended for exposition (Prometheus, OTLP): prefer this over chaining
    /// [`Self::count`], [`Self::sum`], and [`Self::buckets`], which are
    /// independent relaxed loads and may not match one another under concurrent
    /// [`Self::observe`] calls.
    #[must_use]
    pub fn snapshot(&self) -> HistogramSnapshot {
        loop {
            let phase = self.seq.load(Ordering::SeqCst);
            if !phase.is_multiple_of(2) {
                spin_loop();
                continue;
            }
            fence(Ordering::SeqCst);

            let per_bucket: Vec<u64> = self
                .counts
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .collect();
            let observation_count = self.total_count.load(Ordering::Relaxed);
            let sum_value = f64::from_bits(self.sum_bits.load(Ordering::Relaxed));

            fence(Ordering::SeqCst);
            let phase_after = self.seq.load(Ordering::SeqCst);
            let bucket_sum: u64 = per_bucket.iter().sum();
            // Under rare CPU reordering, `phase` can match while bucket/total loads still
            // tear; reject and retry (cheap compared to mutex block on `observe`).
            if phase == phase_after && phase.is_multiple_of(2) && bucket_sum == observation_count {
                return HistogramSnapshot {
                    boundaries: Arc::clone(&self.boundaries),
                    per_bucket: per_bucket.into_boxed_slice(),
                    observation_count,
                    sum_value,
                };
            }
        }
    }

    /// Number of observations recorded.
    #[must_use]
    pub fn count(&self) -> usize {
        self.total_count.load(Ordering::Relaxed) as usize
    }

    /// Sum of all observations.
    #[must_use]
    pub fn sum(&self) -> f64 {
        f64::from_bits(self.sum_bits.load(Ordering::Relaxed))
    }

    /// Returns cumulative bucket counts as `(upper_bound, cumulative_count)` pairs.
    ///
    /// The final entry has `upper_bound = f64::INFINITY`.
    ///
    /// Under concurrent [`Self::observe`] calls, each counter is read with relaxed
    /// ordering; the vector may not agree with [`Self::count`] or [`Self::sum`]
    /// for the same invocation. Use [`Self::snapshot`] for a consistent export view.
    #[must_use]
    pub fn buckets(&self) -> Vec<(f64, u64)> {
        let mut cumulative = 0u64;
        let mut result = Vec::with_capacity(self.counts.len());
        for (i, count) in self.counts.iter().enumerate() {
            cumulative += count.load(Ordering::Relaxed);
            let upper = if i < self.boundaries.len() {
                self.boundaries[i]
            } else {
                f64::INFINITY
            };
            result.push((upper, cumulative));
        }
        result
    }

    /// Count observations that fall at or below each provided boundary.
    ///
    /// Returns `(upper_bound, cumulative_count)` pairs sorted by boundary.
    #[must_use]
    pub fn bucket_counts(&self, boundaries: &[f64]) -> Vec<(f64, u64)> {
        let mut sorted_bounds: Vec<f64> = boundaries.to_vec();
        sorted_bounds.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let cumulative_buckets = self.buckets();

        sorted_bounds
            .iter()
            .map(|&bound| {
                let mut count = 0u64;
                for (upper, cumulative) in &cumulative_buckets {
                    count = *cumulative;
                    if *upper >= bound {
                        break;
                    }
                }
                (bound, count)
            })
            .collect()
    }

    /// Estimate the value at the given percentile using linear interpolation
    /// within buckets (Prometheus-compatible bucketing).
    ///
    /// Returns [`None`] when there are no observations, when `p` is outside
    /// `0.0..=1.0`, or when `p` is not finite.
    #[must_use]
    pub fn percentile(&self, p: f64) -> Option<f64> {
        if !p.is_finite() || !(0.0..=1.0).contains(&p) {
            return None;
        }

        let total = self.total_count.load(Ordering::Relaxed);
        if total == 0 {
            return None;
        }

        let target = p * total as f64;
        let mut cumulative = 0u64;
        let mut prev_bound = 0.0_f64;

        for (i, count) in self.counts.iter().enumerate() {
            let bucket_count = count.load(Ordering::Relaxed);
            cumulative += bucket_count;

            if cumulative as f64 >= target {
                let upper = if i < self.boundaries.len() {
                    self.boundaries[i]
                } else {
                    // +Inf bucket: use the last finite boundary as approximation.
                    prev_bound
                };

                if bucket_count == 0 {
                    return Some(upper);
                }

                // Linear interpolation within the bucket.
                let prev_cumulative = cumulative - bucket_count;
                let fraction = (target - prev_cumulative as f64) / bucket_count as f64;
                return Some((upper - prev_bound).mul_add(fraction, prev_bound));
            }

            if i < self.boundaries.len() {
                prev_bound = self.boundaries[i];
            }
        }

        // Fallback: return last boundary.
        Some(self.boundaries.last().copied().unwrap_or(0.0))
    }

    /// Milliseconds since Unix epoch of the last observation recorded.
    #[must_use]
    pub fn last_updated_ms(&self) -> u64 {
        self.last_updated_ms.load(Ordering::Relaxed)
    }

    /// Upper-bound boundaries configured for this histogram (excludes +Inf).
    ///
    /// Useful for callers that need to verify that a pre-existing histogram
    /// series matches the bucket layout they expect.
    #[must_use]
    pub fn boundaries(&self) -> &[f64] {
        &self.boundaries
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

/// Stored series for one `(metric name, label set)` identity.
#[derive(Debug, Clone)]
enum MetricSeries {
    Counter(Counter),
    Gauge(Gauge),
    Histogram(Histogram),
}

impl MetricSeries {
    fn kind(&self) -> MetricKind {
        match self {
            Self::Counter(_) => MetricKind::Counter,
            Self::Gauge(_) => MetricKind::Gauge,
            Self::Histogram(_) => MetricKind::Histogram,
        }
    }

    fn last_updated_ms(&self) -> u64 {
        match self {
            Self::Counter(c) => c.last_updated_ms(),
            Self::Gauge(g) => g.last_updated_ms(),
            Self::Histogram(h) => h.last_updated_ms(),
        }
    }
}

/// Registry for creating and retrieving named metrics.
///
/// Metric names are interned via [`LabelInterner`] (backed by
/// [`lasso::ThreadedRodeo`]) so repeated lookups for the same name pay only
/// an integer comparison after the first call, not a string allocation.
///
/// Naming conventions and export formats live in `nebula-metrics`, not here.
///
/// Concurrent access uses [`DashMap`] — a sharded lock-free map — so
/// recording metrics on hot paths does not block other threads.
///
/// # Snapshots
///
/// [`Self::snapshot_counters`] and [`Self::snapshot_gauges`] return **live handles**
/// and enumerate registry membership approximately; values may change after the
/// call returns (weak / best-effort view).
///
/// For histograms exposed from [`Self::snapshot_histograms`], use
/// [`Histogram::snapshot`] before export so count, sum, and bucket totals reflect
/// one logical scrape instant (seqlock-backed); the returned [`Histogram`] handle
/// remains live afterward.
///
/// # Examples
///
/// ```
/// use nebula_metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let counter = registry.counter("request_total").unwrap();
/// counter.inc();
/// assert_eq!(counter.get(), 1);
///
/// let same = registry.counter("request_total").unwrap();
/// assert_eq!(same.get(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    /// Shared string interner — both metric names and label keys/values.
    pub(crate) interner: LabelInterner,
    series: Arc<DashMap<MetricKey, MetricSeries>>,
}

impl MetricsRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            interner: LabelInterner::new(),
            series: Arc::new(DashMap::new()),
        }
    }

    fn resolve_metric_name(&self, name: Spur) -> String {
        self.interner.resolve(name).to_owned()
    }

    fn kind_conflict(&self, name: Spur, expected: MetricKind, actual: MetricKind) -> MetricsError {
        MetricsError::MetricKindConflict {
            metric_name: self.resolve_metric_name(name),
            expected_kind: expected,
            actual_kind: actual,
        }
    }

    /// Access the underlying label interner.
    ///
    /// Use this to build [`LabelSet`]s that are compatible with the labeled
    /// metric accessors on this registry.
    #[must_use]
    pub fn interner(&self) -> &LabelInterner {
        &self.interner
    }

    // ── Unlabeled accessors ─────────────────────────────────────────────────

    /// Get or create an unlabeled counter by name.
    pub fn counter(&self, name: &str) -> MetricsResult<Counter> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::unlabeled(name_spur);
        self.insert_counter(key, name_spur)
    }

    /// Get or create an unlabeled gauge by name.
    pub fn gauge(&self, name: &str) -> MetricsResult<Gauge> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::unlabeled(name_spur);
        self.insert_gauge(key, name_spur)
    }

    /// Get or create an unlabeled histogram using the built-in default bucket layout.
    pub fn histogram(&self, name: &str) -> MetricsResult<Histogram> {
        self.histogram_with_buckets_unlabeled(name, DEFAULT_BUCKETS)
    }

    fn histogram_with_buckets_unlabeled(
        &self,
        name: &str,
        boundaries: &[f64],
    ) -> MetricsResult<Histogram> {
        Histogram::validate_bucket_boundaries(boundaries)?;
        let name_spur = self.interner.intern(name);
        let key = MetricKey::unlabeled(name_spur);
        self.insert_histogram(key, name_spur, boundaries)
    }

    // ── Labeled accessors ───────────────────────────────────────────────────

    /// Get or create a counter for the given metric name and label set.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_metrics::MetricsRegistry;
    ///
    /// let reg = MetricsRegistry::new();
    /// let labels = reg.interner().label_set(&[("action_type", "http.request")]);
    /// let counter = reg
    ///     .counter_labeled("action_executions_total", &labels)
    ///     .unwrap();
    /// counter.inc();
    /// assert_eq!(counter.get(), 1);
    /// ```
    pub fn counter_labeled(&self, name: &str, labels: &LabelSet) -> MetricsResult<Counter> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::labeled(name_spur, labels.clone());
        self.insert_counter(key, name_spur)
    }

    /// Get or create a gauge for the given metric name and label set.
    pub fn gauge_labeled(&self, name: &str, labels: &LabelSet) -> MetricsResult<Gauge> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::labeled(name_spur, labels.clone());
        self.insert_gauge(key, name_spur)
    }

    /// Get or create a histogram for the given metric name and label set using
    /// the built-in default bucket layout.
    pub fn histogram_labeled(&self, name: &str, labels: &LabelSet) -> MetricsResult<Histogram> {
        self.histogram_with_buckets_labeled(name, labels, DEFAULT_BUCKETS.to_vec())
    }

    /// Get or create a histogram with custom bucket boundaries and a label set.
    ///
    /// Bucket boundaries are pinned at **first registration**. A later call
    /// with the same `(name, labels)` but different `boundaries` returns
    /// [`MetricsError::HistogramLayoutConflict`].
    pub fn histogram_with_buckets_labeled(
        &self,
        name: &str,
        labels: &LabelSet,
        boundaries: Vec<f64>,
    ) -> MetricsResult<Histogram> {
        Histogram::validate_bucket_boundaries(&boundaries)?;
        let name_spur = self.interner.intern(name);
        let key = MetricKey::labeled(name_spur, labels.clone());
        self.insert_histogram(key, name_spur, &boundaries)
    }

    fn insert_counter(&self, key: MetricKey, name_spur: Spur) -> MetricsResult<Counter> {
        match self.series.entry(key) {
            Entry::Occupied(o) => match o.get() {
                MetricSeries::Counter(c) => Ok(c.clone()),
                other => Err(self.kind_conflict(name_spur, MetricKind::Counter, other.kind())),
            },
            Entry::Vacant(v) => {
                let c = Counter::new();
                v.insert(MetricSeries::Counter(c.clone()));
                Ok(c)
            },
        }
    }

    fn insert_gauge(&self, key: MetricKey, name_spur: Spur) -> MetricsResult<Gauge> {
        match self.series.entry(key) {
            Entry::Occupied(o) => match o.get() {
                MetricSeries::Gauge(g) => Ok(g.clone()),
                other => Err(self.kind_conflict(name_spur, MetricKind::Gauge, other.kind())),
            },
            Entry::Vacant(v) => {
                let g = Gauge::new();
                v.insert(MetricSeries::Gauge(g.clone()));
                Ok(g)
            },
        }
    }

    fn insert_histogram(
        &self,
        key: MetricKey,
        name_spur: Spur,
        boundaries: &[f64],
    ) -> MetricsResult<Histogram> {
        match self.series.entry(key) {
            Entry::Occupied(o) => match o.get() {
                MetricSeries::Histogram(h) => {
                    if h.boundaries() != boundaries {
                        return Err(MetricsError::HistogramLayoutConflict {
                            metric_name: self.resolve_metric_name(name_spur),
                        });
                    }
                    Ok(h.clone())
                },
                other => Err(self.kind_conflict(name_spur, MetricKind::Histogram, other.kind())),
            },
            Entry::Vacant(v) => {
                let h = Histogram::from_validated_boundaries(boundaries.to_vec());
                v.insert(MetricSeries::Histogram(h.clone()));
                Ok(h)
            },
        }
    }

    // ── Snapshot / export ───────────────────────────────────────────────────

    /// Iterate all counter entries as `(MetricKey, Counter)` pairs.
    ///
    /// Used by exporters (Prometheus, OTLP) to serialize the current state.
    pub fn snapshot_counters(&self) -> Vec<(MetricKey, Counter)> {
        self.series
            .iter()
            .filter_map(|entry| match entry.value() {
                MetricSeries::Counter(c) => Some((entry.key().clone(), c.clone())),
                _ => None,
            })
            .collect()
    }

    /// Iterate all gauge entries as `(MetricKey, Gauge)` pairs.
    pub fn snapshot_gauges(&self) -> Vec<(MetricKey, Gauge)> {
        self.series
            .iter()
            .filter_map(|entry| match entry.value() {
                MetricSeries::Gauge(g) => Some((entry.key().clone(), g.clone())),
                _ => None,
            })
            .collect()
    }

    /// Iterate all histogram entries as `(MetricKey, Histogram)` pairs.
    pub fn snapshot_histograms(&self) -> Vec<(MetricKey, Histogram)> {
        self.series
            .iter()
            .filter_map(|entry| match entry.value() {
                MetricSeries::Histogram(h) => Some((entry.key().clone(), h.clone())),
                _ => None,
            })
            .collect()
    }

    // ── Expiration ──────────────────────────────────────────────────────────

    /// Remove all metric series that have not been updated within `max_age`.
    ///
    /// Reclaims stale series so dynamic labels do not grow the registry without
    /// bound. Stable metrics are naturally retained because they are written
    /// continuously.
    ///
    /// # Memory behavior
    ///
    /// After pruning, this compacts the label interner from still-live keys.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use nebula_metrics::MetricsRegistry;
    ///
    /// let mut reg = MetricsRegistry::new();
    /// let c = reg.counter("actions_total").unwrap();
    /// c.inc();
    ///
    /// reg.retain_recent(Duration::from_secs(300));
    /// assert_eq!(reg.metric_count(), 1);
    /// ```
    pub fn retain_recent(&mut self, max_age: Duration) {
        let age_ms: u64 = max_age.as_millis().try_into().unwrap_or(u64::MAX);
        let cutoff_ms = now_ms().saturating_sub(age_ms);
        self.series
            .retain(|_, series| series.last_updated_ms() >= cutoff_ms);
        self.compact_interner();
    }

    /// Total number of tracked metric series.
    #[must_use]
    pub fn metric_count(&self) -> usize {
        self.series.len()
    }

    /// Number of distinct strings held by the label interner.
    #[must_use]
    pub fn interner_len(&self) -> usize {
        self.interner.len()
    }

    fn compact_interner(&mut self) {
        let old_interner = self.interner.clone();
        let new_interner = LabelInterner::new();

        let remap_key = |key: &MetricKey| {
            let name = old_interner.resolve(key.name).to_owned();
            let labels_owned: Vec<(String, String)> = key
                .labels
                .resolve(&old_interner)
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            let label_refs: Vec<(&str, &str)> = labels_owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            MetricKey::labeled(
                new_interner.intern(&name),
                new_interner.label_set(&label_refs),
            )
        };

        let new_series: DashMap<MetricKey, MetricSeries> = DashMap::new();
        for entry in &*self.series {
            let new_key = remap_key(entry.key());
            let cloned = match entry.value() {
                MetricSeries::Counter(c) => MetricSeries::Counter(c.clone()),
                MetricSeries::Gauge(g) => MetricSeries::Gauge(g.clone()),
                MetricSeries::Histogram(h) => MetricSeries::Histogram(h.clone()),
            };
            new_series.insert(new_key, cloned);
        }

        self.interner = new_interner;
        self.series = Arc::new(new_series);
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetricKind, MetricsError};

    #[test]
    fn default_bucket_table_is_valid() {
        assert!(Histogram::validate_bucket_boundaries(DEFAULT_BUCKETS).is_ok());
    }

    #[test]
    fn counter_starts_at_zero() {
        let c = Counter::new();
        assert_eq!(c.get(), 0);
    }

    #[test]
    fn counter_increments() {
        let c = Counter::new();
        c.inc();
        c.inc_by(5);
        assert_eq!(c.get(), 6);
    }

    #[test]
    fn gauge_up_and_down() {
        let g = Gauge::new();
        g.inc();
        g.inc();
        g.dec();
        assert_eq!(g.get(), 1);
        g.set(42);
        assert_eq!(g.get(), 42);
    }

    #[test]
    fn histogram_records_observations() {
        let h = Histogram::new();
        h.observe(1.0);
        h.observe(2.5);
        h.observe(3.0);
        assert_eq!(h.count(), 3);
        assert!((h.sum() - 6.5).abs() < f64::EPSILON);
    }

    #[test]
    fn histogram_default_buckets() {
        let h = Histogram::new();
        let buckets = h.buckets();
        // 11 default boundaries + 1 for +Inf = 12 entries.
        assert_eq!(buckets.len(), 12);
        assert_eq!(buckets.last().unwrap().0, f64::INFINITY);
    }

    #[test]
    fn histogram_custom_buckets() {
        let h = Histogram::try_with_buckets(vec![1.0, 5.0, 10.0]).unwrap();
        let buckets = h.buckets();
        assert_eq!(buckets.len(), 4); // 3 boundaries + +Inf
        assert_eq!(buckets[0].0, 1.0);
        assert_eq!(buckets[1].0, 5.0);
        assert_eq!(buckets[2].0, 10.0);
        assert_eq!(buckets[3].0, f64::INFINITY);
    }

    #[test]
    fn histogram_observe_updates_correct_bucket() {
        let h = Histogram::try_with_buckets(vec![1.0, 5.0, 10.0]).unwrap();
        h.observe(0.5); // bucket 0 (le=1.0)
        h.observe(3.0); // bucket 1 (le=5.0)
        h.observe(7.0); // bucket 2 (le=10.0)
        h.observe(15.0); // bucket 3 (+Inf)

        let buckets = h.buckets();
        // Cumulative counts.
        assert_eq!(buckets[0].1, 1); // le=1.0: 1 obs
        assert_eq!(buckets[1].1, 2); // le=5.0: 1+1
        assert_eq!(buckets[2].1, 3); // le=10.0: 1+1+1
        assert_eq!(buckets[3].1, 4); // +Inf: all 4
    }

    #[test]
    fn histogram_count_and_sum_accurate() {
        let h = Histogram::new();
        h.observe(0.1);
        h.observe(0.2);
        h.observe(0.3);
        assert_eq!(h.count(), 3);
        assert!((h.sum() - 0.6).abs() < 1e-10);
    }

    #[test]
    fn histogram_percentile_basic() {
        let h = Histogram::try_with_buckets(vec![1.0, 5.0, 10.0]).unwrap();
        for _ in 0..50 {
            h.observe(0.5); // bucket [0, 1.0]
        }
        for _ in 0..30 {
            h.observe(3.0); // bucket (1.0, 5.0]
        }
        for _ in 0..20 {
            h.observe(7.0); // bucket (5.0, 10.0]
        }

        let p50 = h.percentile(0.5).expect("p50");
        assert!(p50 <= 1.0, "p50 should be in first bucket, got {p50}");

        let p95 = h.percentile(0.95).expect("p95");
        assert!(p95 > 5.0, "p95 should be in third bucket, got {p95}");
    }

    #[test]
    fn histogram_percentile_empty() {
        let h = Histogram::new();
        assert!(h.percentile(0.5).is_none());
    }

    #[test]
    fn histogram_percentile_single_observation() {
        let h = Histogram::try_with_buckets(vec![1.0, 5.0, 10.0]).unwrap();
        h.observe(3.0);
        let p = h.percentile(1.0).expect("p100");
        // Single observation in (1.0, 5.0] bucket.
        assert!(p > 0.0, "percentile of single observation should be > 0");
    }

    #[test]
    fn histogram_constant_memory() {
        let h = Histogram::new();
        // Observe 1M values — memory should not grow.
        for i in 0..1_000_000 {
            h.observe(i as f64 * 0.001);
        }
        assert_eq!(h.count(), 1_000_000);
        // If this were Vec<f64>, it would use ~8MB. Buckets use < 200 bytes.
    }

    #[test]
    fn histogram_snapshot_sum_of_buckets_equals_total_count() {
        let h = Histogram::try_with_buckets(vec![1.0, 5.0, 10.0]).unwrap();
        for _ in 0..7 {
            h.observe(0.5);
        }
        for _ in 0..11 {
            h.observe(3.0);
        }
        h.observe(100.0);
        let snap = h.snapshot();
        let bucket_sum: u64 = snap.per_bucket_counts().iter().sum();
        assert_eq!(bucket_sum, snap.observation_count());
        assert_eq!(snap.observation_count(), h.count() as u64);
    }

    #[test]
    fn histogram_snapshot_consistent_under_concurrent_observe() {
        use std::sync::{
            Arc as StdArc,
            atomic::{AtomicBool, Ordering as AtomicOrdering},
        };

        let reg = MetricsRegistry::new();
        let h = StdArc::new(reg.histogram("lat").unwrap());
        let stop = StdArc::new(AtomicBool::new(false));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let h = StdArc::clone(&h);
                let stop = StdArc::clone(&stop);
                std::thread::spawn(move || {
                    while !stop.load(AtomicOrdering::Relaxed) {
                        for v in &[0.01_f64, 0.05, 0.2, 2.5, 9.9] {
                            h.observe(*v);
                        }
                    }
                })
            })
            .collect();

        // Keep bounded for `nextest` `agent` profile (pre-push: 30s × 2 slow ceiling).
        for _ in 0..256 {
            let snap = h.snapshot();
            let bucket_sum: u64 = snap.per_bucket_counts().iter().sum();
            assert_eq!(
                bucket_sum,
                snap.observation_count(),
                "per-bucket tally must equal total observations in a snapshot"
            );
            let last_cumulative = snap
                .cumulative_buckets()
                .last()
                .map(|(_, c)| *c)
                .unwrap_or(0);
            assert_eq!(
                last_cumulative,
                snap.observation_count(),
                "+Inf cumulative must equal total count"
            );
        }

        stop.store(true, AtomicOrdering::Relaxed);
        for t in threads {
            let _ = t.join();
        }
    }

    #[test]
    fn histogram_concurrent_observe() {
        use std::sync::Arc as StdArc;

        let h = StdArc::new(Histogram::new());
        let threads: Vec<_> = (0..100)
            .map(|_| {
                let h = StdArc::clone(&h);
                std::thread::spawn(move || {
                    for i in 0..1000 {
                        h.observe(i as f64 * 0.01);
                    }
                })
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }

        assert_eq!(h.count(), 100_000);
    }

    #[test]
    fn histogram_empty_buckets_rejected() {
        assert!(matches!(
            Histogram::try_with_buckets(vec![]),
            Err(MetricsError::InvalidHistogramBuckets { .. })
        ));
    }

    #[test]
    fn histogram_unsorted_buckets_rejected() {
        assert!(matches!(
            Histogram::try_with_buckets(vec![5.0, 1.0, 10.0]),
            Err(MetricsError::InvalidHistogramBuckets { .. })
        ));
    }

    #[test]
    fn registry_returns_same_metric_for_same_name() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("requests").unwrap();
        c1.inc();
        let c2 = reg.counter("requests").unwrap();
        assert_eq!(c2.get(), 1);
    }

    #[test]
    fn registry_rejects_metric_kind_conflict() {
        let reg = MetricsRegistry::new();
        reg.counter("dup").unwrap().inc();
        let err = reg.gauge("dup").unwrap_err();
        assert!(matches!(
            err,
            MetricsError::MetricKindConflict {
                expected_kind: MetricKind::Gauge,
                actual_kind: MetricKind::Counter,
                ..
            }
        ));
    }

    #[test]
    fn registry_different_names_are_independent() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("a").unwrap();
        let c2 = reg.counter("b").unwrap();
        c1.inc();
        assert_eq!(c1.get(), 1);
        assert_eq!(c2.get(), 0);
    }

    #[test]
    fn registry_labeled_counter_independent_from_unlabeled() {
        let reg = MetricsRegistry::new();
        let labels = reg.interner().label_set(&[("env", "prod")]);
        let labeled = reg.counter_labeled("requests_total", &labels).unwrap();
        let unlabeled = reg.counter("requests_total").unwrap();

        labeled.inc_by(10);
        unlabeled.inc_by(3);

        assert_eq!(labeled.get(), 10);
        assert_eq!(unlabeled.get(), 3);
    }

    #[test]
    fn registry_different_label_sets_are_independent() {
        let reg = MetricsRegistry::new();
        let prod = reg.interner().label_set(&[("env", "prod")]);
        let staging = reg.interner().label_set(&[("env", "staging")]);

        let c_prod = reg.counter_labeled("executions_total", &prod).unwrap();
        let c_staging = reg.counter_labeled("executions_total", &staging).unwrap();
        c_prod.inc_by(5);
        c_staging.inc_by(2);

        assert_eq!(c_prod.get(), 5);
        assert_eq!(c_staging.get(), 2);
    }

    #[test]
    fn registry_same_label_set_different_order_returns_same_metric() {
        let reg = MetricsRegistry::new();
        let ls1 = reg
            .interner()
            .label_set(&[("status", "ok"), ("action", "http.request")]);
        let ls2 = reg
            .interner()
            .label_set(&[("action", "http.request"), ("status", "ok")]);

        let c1 = reg
            .counter_labeled("action_executions_total", &ls1)
            .unwrap();
        c1.inc_by(7);
        let c2 = reg
            .counter_labeled("action_executions_total", &ls2)
            .unwrap();

        assert_eq!(
            c2.get(),
            7,
            "same label set in different order must return same metric"
        );
    }

    #[test]
    fn snapshot_returns_all_registered_metrics() {
        let reg = MetricsRegistry::new();
        reg.counter("c1").unwrap().inc();
        reg.counter("c2").unwrap().inc_by(3);
        reg.gauge("g1").unwrap().set(99);
        reg.histogram("h1").unwrap().observe(1.0);

        assert_eq!(reg.snapshot_counters().len(), 2);
        assert_eq!(reg.snapshot_gauges().len(), 1);
        assert_eq!(reg.snapshot_histograms().len(), 1);
    }

    #[test]
    fn metric_count_sums_all_types() {
        let reg = MetricsRegistry::new();
        reg.counter("c1").unwrap().inc();
        reg.gauge("g1").unwrap().set(1);
        reg.histogram("h1").unwrap().observe(1.0);
        assert_eq!(reg.metric_count(), 3);
    }

    #[test]
    fn histogram_with_buckets_labeled_errors_on_layout_conflict() {
        let reg = MetricsRegistry::new();
        let labels = reg.interner().label_set(&[("route", "/health")]);

        let first = reg
            .histogram_with_buckets_labeled("req_latency", &labels, vec![0.1, 0.5, 1.0])
            .unwrap();
        assert_eq!(first.boundaries(), &[0.1, 0.5, 1.0]);

        let err = reg
            .histogram_with_buckets_labeled("req_latency", &labels, vec![2.0, 4.0, 8.0])
            .unwrap_err();
        assert!(matches!(err, MetricsError::HistogramLayoutConflict { .. }));

        first.observe(0.3);
        let second = reg
            .histogram_with_buckets_labeled("req_latency", &labels, vec![0.1, 0.5, 1.0])
            .unwrap();
        assert_eq!(second.count(), 1);
    }

    #[test]
    fn histogram_boundaries_accessor_excludes_inf() {
        let h = Histogram::try_with_buckets(vec![1.0, 2.0, 3.0]).unwrap();
        assert_eq!(h.boundaries(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn interner_len_compacts_after_retain_recent() {
        let mut reg = MetricsRegistry::new();
        let labels = reg.interner().label_set(&[("k", "v1")]);
        reg.counter_labeled("m", &labels).unwrap().inc();
        let before = reg.interner_len();
        assert!(before >= 3); // at least "m", "k", "v1"

        std::thread::sleep(Duration::from_millis(2));
        reg.retain_recent(Duration::ZERO);
        assert_eq!(reg.metric_count(), 0);
        // Compaction rebuilds the interner from live series (none left).
        assert_eq!(reg.interner_len(), 0);
    }

    #[test]
    fn retain_recent_keeps_recently_updated_metrics() {
        let mut reg = MetricsRegistry::new();
        reg.counter("fresh").unwrap().inc();
        reg.gauge("also_fresh").unwrap().set(1);

        // Everything was just written — a 1-hour window should keep all.
        reg.retain_recent(Duration::from_hours(1));
        assert_eq!(reg.metric_count(), 2);
    }

    #[test]
    fn retain_recent_removes_stale_metrics() {
        let mut reg = MetricsRegistry::new();

        // Register a counter and immediately call retain_recent with zero
        // max_age so the cutoff is `now_ms()`.  Any metric whose timestamp
        // is strictly less than the cutoff will be removed.  Since we just
        // created the metrics they will have timestamps >= cutoff, so this
        // verifies the boundary condition: nothing is removed at max_age = 0.
        reg.counter("a").unwrap().inc();
        reg.gauge("b").unwrap().set(1);
        reg.histogram("c").unwrap().observe(0.5);
        reg.retain_recent(Duration::from_hours(1));
        assert_eq!(
            reg.metric_count(),
            3,
            "all metrics are recent, none should be removed"
        );

        // Now retain with max_age = 0: cutoff = now_ms(), so only metrics
        // updated at exactly now_ms() or later survive.  In practice all
        // metrics were created a few microseconds ago which means
        // their timestamp < cutoff, and they will be evicted.
        // We sleep 1 ms to ensure the cutoff is strictly after creation time.
        std::thread::sleep(Duration::from_millis(2));
        reg.retain_recent(Duration::ZERO);
        assert_eq!(
            reg.metric_count(),
            0,
            "all metrics should be evicted when max_age = 0"
        );
    }
}
