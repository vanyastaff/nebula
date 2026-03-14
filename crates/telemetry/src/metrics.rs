//! Metrics primitives and registry.
//!
//! Provides lightweight metric types (counter, gauge, histogram) and a
//! registry to create and retrieve them. The MVP implementation stores
//! values in-memory with atomics -- no external exporter needed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tracing::warn;

/// An incrementing counter.
#[derive(Debug, Clone)]
pub struct Counter {
    value: Arc<AtomicU64>,
}

impl Counter {
    /// Create a new counter starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment by a given amount.
    pub fn inc_by(&self, n: u64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
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
}

impl Gauge {
    /// Create a new gauge starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement by one.
    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }

    /// Set to a specific value.
    pub fn set(&self, v: i64) {
        self.value.store(v, Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
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
}

impl Clone for Histogram {
    fn clone(&self) -> Self {
        Self {
            boundaries: Arc::clone(&self.boundaries),
            counts: Arc::clone(&self.counts),
            total_count: Arc::clone(&self.total_count),
            sum_bits: Arc::clone(&self.sum_bits),
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
        }
    }

    /// Record an observation.
    pub fn observe(&self, value: f64) {
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
    counters: Arc<RwLock<HashMap<String, Counter>>>,
    gauges: Arc<RwLock<HashMap<String, Gauge>>>,
    histograms: Arc<RwLock<HashMap<String, Histogram>>>,
}

impl MetricsRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            counters: Arc::new(RwLock::new(HashMap::new())),
            gauges: Arc::new(RwLock::new(HashMap::new())),
            histograms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a counter by name.
    pub fn counter(&self, name: &str) -> Counter {
        let mut map = self.counters.write().unwrap_or_else(|poisoned| {
            warn!("counter registry lock was poisoned, recovering");
            poisoned.into_inner()
        });
        map.entry(name.to_owned()).or_default().clone()
    }

    /// Get or create a gauge by name.
    pub fn gauge(&self, name: &str) -> Gauge {
        let mut map = self.gauges.write().unwrap_or_else(|poisoned| {
            warn!("gauge registry lock was poisoned, recovering");
            poisoned.into_inner()
        });
        map.entry(name.to_owned()).or_default().clone()
    }

    /// Get or create a histogram by name.
    pub fn histogram(&self, name: &str) -> Histogram {
        let mut map = self.histograms.write().unwrap_or_else(|poisoned| {
            warn!("histogram registry lock was poisoned, recovering");
            poisoned.into_inner()
        });
        map.entry(name.to_owned()).or_default().clone()
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A no-op metrics registry that discards all observations.
///
/// Useful for testing and contexts where metrics are not needed.
#[derive(Debug, Clone, Copy)]
pub struct NoopMetricsRegistry;

impl NoopMetricsRegistry {
    /// Create a noop registry.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopMetricsRegistry {
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
    fn registry_recovers_from_poisoned_lock() {
        let reg = MetricsRegistry::new();
        reg.counter("before");

        // Poison the counters lock.
        let inner = reg.counters.clone();
        let handle = std::thread::spawn(move || {
            let _guard = inner.write().unwrap();
            panic!("intentional panic to poison registry lock");
        });
        assert!(handle.join().is_err());

        // After poisoning, counter/gauge/histogram creation must recover.
        let c = reg.counter("after");
        c.inc();
        assert_eq!(c.get(), 1);

        let g = reg.gauge("g");
        g.set(42);
        assert_eq!(g.get(), 42);

        let h = reg.histogram("h");
        h.observe(1.0);
        assert_eq!(h.count(), 1);
    }
}
