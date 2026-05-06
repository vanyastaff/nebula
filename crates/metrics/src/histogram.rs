//! Histogram primitive with fixed bucket layout.
//!
//! Lock-free hot path: [`Histogram::observe`] uses a seqlock bracket
//! (sequentially consistent phase counter + fences) so the observation count,
//! sum, and per-bucket tallies remain mutually consistent for [`Histogram::snapshot`]
//! consumers without blocking concurrent writers. Sum is stored as `f64` bits
//! in an [`std::sync::atomic::AtomicU64`] and updated via [`AtomicU64::update`]
//! (Rust 1.95).

use std::{
    hint::spin_loop,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering, fence},
    },
};

use crate::{
    error::{MetricsError, MetricsResult},
    registry::now_ms,
};

/// Default finite upper bounds for the built-in histogram layout (sub-second
/// through 10 seconds, suited to latency-style measurements in seconds).
pub(crate) const DEFAULT_BUCKETS: &[f64] = &[
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

    pub(crate) fn from_validated_boundaries(boundaries: Vec<f64>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetricsError, registry::MetricsRegistry};

    #[test]
    fn default_bucket_table_is_valid() {
        assert!(Histogram::validate_bucket_boundaries(DEFAULT_BUCKETS).is_ok());
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
    fn histogram_boundaries_accessor_excludes_inf() {
        let h = Histogram::try_with_buckets(vec![1.0, 2.0, 3.0]).unwrap();
        assert_eq!(h.boundaries(), &[1.0, 2.0, 3.0]);
    }
}
