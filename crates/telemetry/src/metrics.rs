//! Metrics primitives and registry.
//!
//! Provides lightweight metric types (counter, gauge, histogram) and a
//! registry to create and retrieve them. The implementation stores values
//! in-memory with atomics and uses [`lasso`]-backed string interning plus
//! [`dashmap`]-backed sharded maps for low-latency concurrent access on the
//! hot recording path.

use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;

use crate::labels::{LabelInterner, LabelSet, MetricKey};

/// Returns the current time as milliseconds since the Unix epoch.
#[inline]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
    pub fn inc_by(&self, n: u64) {
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
    pub fn set(&self, v: i64) {
        self.value.store(v, Ordering::Relaxed);
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
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

/// Default Prometheus histogram bucket boundaries.
const DEFAULT_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// A histogram that records observations into fixed buckets.
///
/// Uses Prometheus-style bucket boundaries with constant memory usage
/// regardless of the number of observations. Each observation increments
/// the appropriate bucket counter atomically (lock-free).
///
/// The default buckets are suited for measuring request durations in seconds.
/// Use [`Histogram::with_buckets`] for custom boundaries.
#[derive(Debug)]
pub struct Histogram {
    /// Upper-bound for each bucket (sorted, does not include +Inf).
    boundaries: Arc<Vec<f64>>,
    /// Cumulative count per bucket (len = boundaries.len() + 1 for +Inf).
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
            boundaries: Arc::clone(&self.boundaries),
            counts: Arc::clone(&self.counts),
            total_count: Arc::clone(&self.total_count),
            sum_bits: Arc::clone(&self.sum_bits),
            last_updated_ms: Arc::clone(&self.last_updated_ms),
        }
    }
}

impl Histogram {
    /// Create a histogram with default Prometheus bucket boundaries.
    #[must_use]
    pub fn new() -> Self {
        Self::with_buckets(DEFAULT_BUCKETS.to_vec())
    }

    /// Create a histogram with custom bucket boundaries.
    ///
    /// Boundaries must be non-empty, all positive, and sorted ascending.
    ///
    /// # Panics
    ///
    /// Panics if `boundaries` is empty, contains non-positive values,
    /// or is not sorted in ascending order.
    #[must_use]
    pub fn with_buckets(boundaries: Vec<f64>) -> Self {
        assert!(
            !boundaries.is_empty(),
            "histogram boundaries must not be empty"
        );
        assert!(
            boundaries.iter().all(|&b| b > 0.0 && b.is_finite()),
            "histogram boundaries must be positive and finite",
        );
        assert!(
            boundaries.windows(2).all(|w| w[0] < w[1]),
            "histogram boundaries must be sorted in ascending order with no duplicates",
        );

        let bucket_count = boundaries.len() + 1; // +1 for +Inf
        let counts: Vec<AtomicU64> = (0..bucket_count).map(|_| AtomicU64::new(0)).collect();

        tracing::debug!(buckets = boundaries.len(), "histogram created");

        Self {
            boundaries: Arc::new(boundaries),
            counts: Arc::new(counts),
            total_count: Arc::new(AtomicU64::new(0)),
            sum_bits: Arc::new(AtomicU64::new(0.0_f64.to_bits())),
            last_updated_ms: Arc::new(AtomicU64::new(now_ms())),
        }
    }

    /// Record an observation.
    ///
    /// Non-finite values (`NaN`, `±∞`) are silently dropped. NaN would
    /// otherwise permanently poison `sum_bits` via the CAS loop
    /// (`x + NaN = NaN`), breaking every subsequent `sum()` / percentile.
    pub fn observe(&self, value: f64) {
        if !value.is_finite() {
            return;
        }

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

        // Atomically add to sum using CAS loop on f64 bits.
        loop {
            let old_bits = self.sum_bits.load(Ordering::Relaxed);
            let old_sum = f64::from_bits(old_bits);
            let new_sum = old_sum + value;
            let new_bits = new_sum.to_bits();
            if self
                .sum_bits
                .compare_exchange_weak(old_bits, new_bits, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
        self.last_updated_ms.store(now_ms(), Ordering::Relaxed);
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

    /// Estimate the value at the given percentile (0.0–1.0) using linear
    /// interpolation within buckets (same approach as Prometheus).
    ///
    /// Returns `0.0` if no observations have been recorded.
    #[must_use]
    pub fn percentile(&self, p: f64) -> f64 {
        let total = self.total_count.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
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
                    return upper;
                }

                // Linear interpolation within the bucket.
                let prev_cumulative = cumulative - bucket_count;
                let fraction = (target - prev_cumulative as f64) / bucket_count as f64;
                return prev_bound + (upper - prev_bound) * fraction;
            }

            if i < self.boundaries.len() {
                prev_bound = self.boundaries[i];
            }
        }

        // Fallback: return last boundary.
        self.boundaries.last().copied().unwrap_or(0.0)
    }

    /// Milliseconds since Unix epoch of the last observation recorded.
    #[must_use]
    pub fn last_updated_ms(&self) -> u64 {
        self.last_updated_ms.load(Ordering::Relaxed)
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry for creating and retrieving named metrics.
///
/// Prefer names with the `nebula_` prefix (e.g. `nebula_executions_total`)
/// for consistency and future Prometheus/OTLP export.
///
/// Metric names are interned via [`LabelInterner`] (backed by
/// [`lasso::ThreadedRodeo`]) so repeated `counter("same_name")` calls pay
/// only an integer comparison after the first call, not a string allocation.
///
/// Concurrent access uses [`DashMap`] — a sharded lock-free map — so
/// recording metrics on hot paths does not block other threads.
///
/// # Examples
///
/// ```
/// use nebula_telemetry::metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let counter = registry.counter("nebula_executions_total");
/// counter.inc();
/// assert_eq!(counter.get(), 1);
///
/// // Retrieving the same name returns the same metric.
/// let same = registry.counter("nebula_executions_total");
/// assert_eq!(same.get(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    /// Shared string interner — both metric names and label keys/values.
    pub(crate) interner: LabelInterner,
    counters: Arc<DashMap<MetricKey, Counter>>,
    gauges: Arc<DashMap<MetricKey, Gauge>>,
    histograms: Arc<DashMap<MetricKey, Histogram>>,
}

impl MetricsRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            interner: LabelInterner::new(),
            counters: Arc::new(DashMap::new()),
            gauges: Arc::new(DashMap::new()),
            histograms: Arc::new(DashMap::new()),
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
    pub fn counter(&self, name: &str) -> Counter {
        let key = MetricKey::unlabeled(self.interner.intern(name));
        self.counters.entry(key).or_default().value().clone()
    }

    /// Get or create an unlabeled gauge by name.
    pub fn gauge(&self, name: &str) -> Gauge {
        let key = MetricKey::unlabeled(self.interner.intern(name));
        self.gauges.entry(key).or_default().value().clone()
    }

    /// Get or create an unlabeled histogram by name.
    pub fn histogram(&self, name: &str) -> Histogram {
        let key = MetricKey::unlabeled(self.interner.intern(name));
        self.histograms.entry(key).or_default().value().clone()
    }

    // ── Labeled accessors ───────────────────────────────────────────────────

    /// Get or create a counter for the given metric name and label set.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_telemetry::metrics::MetricsRegistry;
    ///
    /// let reg = MetricsRegistry::new();
    /// let labels = reg.interner().label_set(&[("action_type", "http.request")]);
    /// let counter = reg.counter_labeled("nebula_action_executions_total", &labels);
    /// counter.inc();
    /// assert_eq!(counter.get(), 1);
    /// ```
    pub fn counter_labeled(&self, name: &str, labels: &LabelSet) -> Counter {
        let key = MetricKey::labeled(self.interner.intern(name), labels.clone());
        self.counters.entry(key).or_default().value().clone()
    }

    /// Get or create a gauge for the given metric name and label set.
    pub fn gauge_labeled(&self, name: &str, labels: &LabelSet) -> Gauge {
        let key = MetricKey::labeled(self.interner.intern(name), labels.clone());
        self.gauges.entry(key).or_default().value().clone()
    }

    /// Get or create a histogram for the given metric name and label set.
    pub fn histogram_labeled(&self, name: &str, labels: &LabelSet) -> Histogram {
        let key = MetricKey::labeled(self.interner.intern(name), labels.clone());
        self.histograms.entry(key).or_default().value().clone()
    }

    /// Get or create a histogram with custom bucket boundaries and a label set.
    pub fn histogram_with_buckets_labeled(
        &self,
        name: &str,
        labels: &LabelSet,
        boundaries: Vec<f64>,
    ) -> Histogram {
        let key = MetricKey::labeled(self.interner.intern(name), labels.clone());
        self.histograms
            .entry(key)
            .or_insert_with(|| Histogram::with_buckets(boundaries))
            .value()
            .clone()
    }

    // ── Snapshot / export ───────────────────────────────────────────────────

    /// Iterate all counter entries as `(MetricKey, Counter)` pairs.
    ///
    /// Used by exporters (Prometheus, OTLP) to serialize the current state.
    pub fn snapshot_counters(&self) -> Vec<(MetricKey, Counter)> {
        self.counters
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Iterate all gauge entries as `(MetricKey, Gauge)` pairs.
    pub fn snapshot_gauges(&self) -> Vec<(MetricKey, Gauge)> {
        self.gauges
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Iterate all histogram entries as `(MetricKey, Histogram)` pairs.
    pub fn snapshot_histograms(&self) -> Vec<(MetricKey, Histogram)> {
        self.histograms
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    // ── Expiration ──────────────────────────────────────────────────────────

    /// Remove all metric series that have not been updated within `max_age`.
    ///
    /// This prevents unbounded memory growth when dynamic labels (e.g.
    /// `action_type`, `workflow_id`) create many high-cardinality series over
    /// time. Stable global metrics (unlabeled or with a fixed label set) are
    /// naturally retained because they are written to continuously.
    ///
    /// Call this periodically from a background task or at the end of a
    /// logical time window (e.g. after completing a batch of executions).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use nebula_telemetry::metrics::MetricsRegistry;
    ///
    /// let reg = MetricsRegistry::new();
    /// let c = reg.counter("nebula_actions_total");
    /// c.inc();
    ///
    /// // Retain everything written in the last 5 minutes.
    /// reg.retain_recent(Duration::from_secs(300));
    /// assert_eq!(reg.metric_count(), 1); // still present — just updated
    /// ```
    pub fn retain_recent(&self, max_age: Duration) {
        let cutoff_ms = now_ms().saturating_sub(max_age.as_millis() as u64);
        self.counters
            .retain(|_, v| v.last_updated_ms() >= cutoff_ms);
        self.gauges.retain(|_, v| v.last_updated_ms() >= cutoff_ms);
        self.histograms
            .retain(|_, v| v.last_updated_ms() >= cutoff_ms);
    }

    /// Total number of tracked metric series (counters + gauges + histograms).
    ///
    /// Useful for cardinality monitoring.
    #[must_use]
    pub fn metric_count(&self) -> usize {
        self.counters.len() + self.gauges.len() + self.histograms.len()
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
        let h = Histogram::with_buckets(vec![1.0, 5.0, 10.0]);
        let buckets = h.buckets();
        assert_eq!(buckets.len(), 4); // 3 boundaries + +Inf
        assert_eq!(buckets[0].0, 1.0);
        assert_eq!(buckets[1].0, 5.0);
        assert_eq!(buckets[2].0, 10.0);
        assert_eq!(buckets[3].0, f64::INFINITY);
    }

    #[test]
    fn histogram_observe_updates_correct_bucket() {
        let h = Histogram::with_buckets(vec![1.0, 5.0, 10.0]);
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
        let h = Histogram::with_buckets(vec![1.0, 5.0, 10.0]);
        for _ in 0..50 {
            h.observe(0.5); // bucket [0, 1.0]
        }
        for _ in 0..30 {
            h.observe(3.0); // bucket (1.0, 5.0]
        }
        for _ in 0..20 {
            h.observe(7.0); // bucket (5.0, 10.0]
        }

        let p50 = h.percentile(0.5);
        assert!(p50 <= 1.0, "p50 should be in first bucket, got {p50}");

        let p95 = h.percentile(0.95);
        assert!(p95 > 5.0, "p95 should be in third bucket, got {p95}");
    }

    #[test]
    fn histogram_percentile_empty() {
        let h = Histogram::new();
        assert!((h.percentile(0.5) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn histogram_percentile_single_observation() {
        let h = Histogram::with_buckets(vec![1.0, 5.0, 10.0]);
        h.observe(3.0);
        let p = h.percentile(1.0);
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
    #[should_panic(expected = "histogram boundaries must not be empty")]
    fn histogram_empty_buckets_panics() {
        let _ = Histogram::with_buckets(vec![]);
    }

    #[test]
    #[should_panic(expected = "histogram boundaries must be sorted")]
    fn histogram_unsorted_buckets_panics() {
        let _ = Histogram::with_buckets(vec![5.0, 1.0, 10.0]);
    }

    #[test]
    fn registry_returns_same_metric_for_same_name() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("requests");
        c1.inc();
        let c2 = reg.counter("requests");
        assert_eq!(c2.get(), 1);
    }

    #[test]
    fn registry_different_names_are_independent() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("a");
        let c2 = reg.counter("b");
        c1.inc();
        assert_eq!(c1.get(), 1);
        assert_eq!(c2.get(), 0);
    }

    #[test]
    fn registry_labeled_counter_independent_from_unlabeled() {
        let reg = MetricsRegistry::new();
        let labels = reg.interner().label_set(&[("env", "prod")]);
        let labeled = reg.counter_labeled("nebula_requests_total", &labels);
        let unlabeled = reg.counter("nebula_requests_total");

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

        let c_prod = reg.counter_labeled("nebula_executions_total", &prod);
        let c_staging = reg.counter_labeled("nebula_executions_total", &staging);
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

        let c1 = reg.counter_labeled("nebula_action_executions_total", &ls1);
        c1.inc_by(7);
        let c2 = reg.counter_labeled("nebula_action_executions_total", &ls2);

        assert_eq!(
            c2.get(),
            7,
            "same label set in different order must return same metric"
        );
    }

    #[test]
    fn snapshot_returns_all_registered_metrics() {
        let reg = MetricsRegistry::new();
        reg.counter("c1").inc();
        reg.counter("c2").inc_by(3);
        reg.gauge("g1").set(99);
        reg.histogram("h1").observe(1.0);

        assert_eq!(reg.snapshot_counters().len(), 2);
        assert_eq!(reg.snapshot_gauges().len(), 1);
        assert_eq!(reg.snapshot_histograms().len(), 1);
    }

    #[test]
    fn metric_count_sums_all_types() {
        let reg = MetricsRegistry::new();
        reg.counter("c1").inc();
        reg.gauge("g1").set(1);
        reg.histogram("h1").observe(1.0);
        assert_eq!(reg.metric_count(), 3);
    }

    #[test]
    fn retain_recent_keeps_recently_updated_metrics() {
        let reg = MetricsRegistry::new();
        reg.counter("fresh").inc();
        reg.gauge("also_fresh").set(1);

        // Everything was just written — a 1-hour window should keep all.
        reg.retain_recent(Duration::from_secs(3600));
        assert_eq!(reg.metric_count(), 2);
    }

    #[test]
    fn retain_recent_removes_stale_metrics() {
        let reg = MetricsRegistry::new();

        // Register a counter and immediately call retain_recent with zero
        // max_age so the cutoff is `now_ms()`.  Any metric whose timestamp
        // is strictly less than the cutoff will be removed.  Since we just
        // created the metrics they will have timestamps >= cutoff, so this
        // verifies the boundary condition: nothing is removed at max_age = 0.
        reg.counter("a").inc();
        reg.gauge("b").set(1);
        reg.histogram("c").observe(0.5);
        reg.retain_recent(Duration::from_secs(3600));
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
