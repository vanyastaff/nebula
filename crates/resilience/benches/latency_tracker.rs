//! Micro-benchmarks for `LatencyTracker` — the sorted-Vec histogram inside
//! `AdaptiveHedgeExecutor`.
//!
//! Measures the two hot paths:
//! - **`record` steady-state** — ring is full; every call evicts the oldest entry and
//!   binary-searches the histogram. With `Vec<(u64, u32)>` this is alloc-free after warmup.
//! - **`percentile`** — walks the sorted histogram to accumulate counts.
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench latency_tracker --features bench
//! ```

use std::hint::black_box;
use std::time::Duration;

use nebula_resilience::BenchLatencyTracker;

fn main() {
    divan::main();
}

// ── record — steady-state (ring full) ─────────────────────────────────────────

/// Benchmark `record()` when the ring is already full.
///
/// In steady-state every call:
/// 1. Pops the oldest nanos value from the ring.
/// 2. `binary_search`es the histogram to decrement (or remove) the evicted bucket.
/// 3. `binary_search`es again to insert/increment the new bucket.
///
/// With `Vec<(u64, u32)>` no heap allocation occurs after the initial warmup.
#[divan::bench(
    name = "record/steady_state",
    args = [10, 100, 500, 1_000],
    sample_count = 500,
)]
fn record_steady_state(bencher: divan::Bencher, max_samples: usize) {
    // Build a tracker with a few distinct latency buckets so the histogram
    // has O(10) entries — realistic for a service with tight latency distribution.
    let mut tracker = BenchLatencyTracker::new(max_samples);
    for i in 0..max_samples {
        // 10 distinct values → histogram stays small regardless of ring size
        tracker.record(Duration::from_nanos((i % 10) as u64 * 100 + 50));
    }

    bencher.bench_local(|| {
        // Rotate through 10 distinct values so eviction + insertion always happen
        tracker.record(black_box(Duration::from_nanos(550)));
        tracker.record(black_box(Duration::from_nanos(650)));
    });
}

/// Same as above but with ALL DISTINCT values — worst case for histogram size.
/// Histogram grows to min(ring_size, unique_values) entries.
#[divan::bench(
    name = "record/all_distinct",
    args = [10, 100, 500],
    sample_count = 500,
)]
fn record_all_distinct(bencher: divan::Bencher, max_samples: usize) {
    let mut tracker = BenchLatencyTracker::new(max_samples);
    for i in 0..max_samples {
        tracker.record(Duration::from_nanos(i as u64 * 100));
    }
    let mut counter = max_samples as u64;
    bencher.bench_local(|| {
        // Each record brings a new unique value, evicts a unique value → histogram stays full
        tracker.record(black_box(Duration::from_nanos(counter * 100)));
        counter += 1;
    });
}

// ── percentile ────────────────────────────────────────────────────────────────

/// Benchmark `percentile(p)` — walks the histogram and accumulates counts.
/// Cost is O(distinct_values), independent of ring size.
#[divan::bench(
    name = "percentile/p99",
    args = [10, 100, 500, 1_000],
    sample_count = 500,
)]
fn percentile_p99(bencher: divan::Bencher, max_samples: usize) {
    let mut tracker = BenchLatencyTracker::new(max_samples);
    for i in 0..max_samples {
        tracker.record(Duration::from_nanos((i % 50) as u64 * 100));
    }
    bencher.bench_local(|| black_box(tracker.percentile(0.99)));
}

/// Benchmark `percentile` across different quantiles on the same filled tracker.
#[divan::bench(
    name = "percentile/multi_quantile",
    consts = [0, 50, 75, 90, 95, 99],
    sample_count = 500,
)]
fn percentile_multi<const P: u32>(bencher: divan::Bencher) {
    const MAX_SAMPLES: usize = 1_000;
    let mut tracker = BenchLatencyTracker::new(MAX_SAMPLES);
    for i in 0..MAX_SAMPLES {
        tracker.record(Duration::from_nanos((i % 100) as u64 * 100));
    }
    let p = P as f64 / 100.0;
    bencher.bench_local(|| black_box(tracker.percentile(p)));
}
