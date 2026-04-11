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
//! cargo bench -p nebula-resilience --bench latency_tracker
//! ```

use std::{hint::black_box, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::LatencyTracker;

// ── record ────────────────────────────────────────────────────────────────────

fn bench_record_steady_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency_tracker/record/steady_state");
    for max_samples in [10usize, 100, 500, 1_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(max_samples),
            &max_samples,
            |b, &n| {
                let mut tracker = LatencyTracker::new(n);
                // warm up — fill the ring so every subsequent record evicts
                for i in 0..n {
                    tracker.record(Duration::from_nanos((i % 10) as u64 * 100 + 50));
                }
                b.iter(|| {
                    tracker.record(black_box(Duration::from_nanos(550)));
                    tracker.record(black_box(Duration::from_nanos(650)));
                });
            },
        );
    }
    group.finish();
}

fn bench_record_all_distinct(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency_tracker/record/all_distinct");
    for max_samples in [10usize, 100, 500] {
        group.bench_with_input(
            BenchmarkId::from_parameter(max_samples),
            &max_samples,
            |b, &n| {
                let mut tracker = LatencyTracker::new(n);
                for i in 0..n {
                    tracker.record(Duration::from_nanos(i as u64 * 100));
                }
                let mut counter = n as u64;
                b.iter(|| {
                    tracker.record(black_box(Duration::from_nanos(counter * 100)));
                    counter += 1;
                });
            },
        );
    }
    group.finish();
}

// ── percentile ────────────────────────────────────────────────────────────────

fn bench_percentile(c: &mut Criterion) {
    let mut group = c.benchmark_group("latency_tracker/percentile");
    for max_samples in [10usize, 100, 500, 1_000] {
        group.bench_with_input(
            BenchmarkId::from_parameter(max_samples),
            &max_samples,
            |b, &n| {
                let mut tracker = LatencyTracker::new(n);
                for i in 0..n {
                    tracker.record(Duration::from_nanos((i % 50) as u64 * 100));
                }
                b.iter(|| black_box(tracker.percentile(0.99)));
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_record_steady_state,
    bench_record_all_distinct,
    bench_percentile,
);
criterion_main!(benches);
