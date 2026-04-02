//! Benchmarks for the hedge pattern — `HedgeExecutor` and `AdaptiveHedgeExecutor`.
//!
//! Three cost layers:
//!
//! - **No-hedge path** (`hedge/call`) — operation succeeds before any hedge delay fires.
//!   Measures `JoinSet::spawn` + one `select!` poll that yields the result immediately.
//!   This is the dominant path in healthy systems where P50 latency < hedge_delay.
//!
//! - **Adaptive overhead** (`hedge/adaptive`) — extra cost of `AdaptiveHedgeExecutor`
//!   over `HedgeExecutor`: `RwLock::read` + `percentile()` walk + `RwLock::write` +
//!   `record()` histogram insert.  Three variants: static baseline, cold tracker
//!   (percentile returns None → fallback), and warmed tracker (full 1000-sample ring).
//!
//! - **Write-lock contention** (`hedge/adaptive/contention`) — concurrent callers fighting
//!   over `AdaptiveHedgeExecutor`'s `RwLock<LatencyTracker>`.  The write lock taken at
//!   call-end for `record()` is the serialisation point; this bench quantifies that cost.
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench hedge
//! ```

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nebula_resilience::hedge::AdaptiveHedgeExecutor;
use nebula_resilience::{HedgeConfig, HedgeExecutor};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// Config that never fires a hedge during a sub-millisecond operation.
///
/// hedge_delay = 10 s >> ~1 µs bench operation, so the `select!` always resolves
/// via the `join_next` arm and the timer is never polled to completion.
fn never_hedge_config() -> HedgeConfig {
    HedgeConfig {
        hedge_delay: Duration::from_secs(10),
        max_hedges: 2,
        exponential_backoff: false,
        backoff_multiplier: 1.0,
    }
}

// ── Bench 1: No-hedge fast path ───────────────────────────────────────────────

/// Measures `HedgeExecutor::call` overhead when the first task succeeds immediately.
///
/// Breakdown: one `JoinSet::spawn` (task allocation), one `select!` iteration that
/// resolves the ready task via `join_next`, `JoinSet::abort_all()` with nothing to
/// abort, and return.  No timer allocation is avoided here — `sleep(10 s)` is created
/// but never polled past the first task completing.
fn bench_call_no_hedge(c: &mut Criterion) {
    let mut group = c.benchmark_group("hedge/call");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(1));

    group.bench_function("no_hedge_needed", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let executor = Arc::new(HedgeExecutor::new(never_hedge_config()).unwrap());

        b.to_async(&rt).iter(|| {
            let executor = Arc::clone(&executor);
            async move {
                let result = executor
                    .call(|| async { Ok::<u64, &str>(black_box(42)) })
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

// ── Bench 2: Adaptive overhead ────────────────────────────────────────────────

/// Compares `AdaptiveHedgeExecutor` to plain `HedgeExecutor` on the no-hedge path.
///
/// Adaptive overhead = `(adaptive_warmed - static_baseline)` and consists of:
/// - `RwLock::read()` + `percentile()` linear walk at call start
/// - `HedgeConfig` clone to build the inner `HedgeExecutor`
/// - `RwLock::write()` + `record()` histogram binary-search insert at call end
///
/// The cold variant exercises the `percentile() → None` branch (tracker empty,
/// falls back to `base_config.hedge_delay`), showing the minimum adaptive cost.
fn bench_adaptive_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("hedge/adaptive");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(1));

    // ── static baseline ────────────────────────────────────────────────────────
    group.bench_function("static_baseline", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let executor = Arc::new(HedgeExecutor::new(never_hedge_config()).unwrap());

        b.to_async(&rt).iter(|| {
            let executor = Arc::clone(&executor);
            async move {
                let result = executor
                    .call(|| async { Ok::<u64, &str>(black_box(42)) })
                    .await;
                black_box(result)
            }
        });
    });

    // ── adaptive, cold tracker ────────────────────────────────────────────────
    // percentile() returns None (ring empty) → falls back to base_config.hedge_delay.
    group.bench_function("adaptive_cold", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let executor = Arc::new(
            AdaptiveHedgeExecutor::new(never_hedge_config())
                .unwrap()
                .with_max_samples(1000)
                .unwrap(),
        );

        b.to_async(&rt).iter(|| {
            let executor = Arc::clone(&executor);
            async move {
                let result = executor
                    .call(|| async { Ok::<u64, &str>(black_box(42)) })
                    .await;
                black_box(result)
            }
        });
    });

    // ── adaptive, warmed tracker ───────────────────────────────────────────────
    // Ring is full (1000 entries); percentile() walks the sorted histogram.
    // Pre-warm is outside iter — 1000 calls do not contribute to the measurement.
    group.bench_function("adaptive_warmed", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let executor = Arc::new(
            AdaptiveHedgeExecutor::new(never_hedge_config())
                .unwrap()
                .with_max_samples(1000)
                .unwrap(),
        );
        rt.block_on(async {
            for _ in 0..1000 {
                let _ = executor.call(|| async { Ok::<u64, &str>(42) }).await;
            }
        });

        b.to_async(&rt).iter(|| {
            let executor = Arc::clone(&executor);
            async move {
                let result = executor
                    .call(|| async { Ok::<u64, &str>(black_box(42)) })
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

// ── Bench 3: max_samples scaling ─────────────────────────────────────────────

/// How does adaptive call cost scale with `max_samples`?
///
/// `LatencyTracker` uses a sorted `Vec<(u64, u32)>` histogram.  `percentile()` is
/// O(k) where k = number of *distinct* latency buckets (≤ max_samples).
/// In steady-state all values come from `Instant::elapsed()` which clusters tightly,
/// so k << max_samples; this bench reflects that by letting the natural distribution
/// form from repeated fast calls.
#[expect(
    clippy::excessive_nesting,
    reason = "criterion async benchmark setup requires nested closure structure"
)]
fn bench_adaptive_sample_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("hedge/adaptive/sample_scaling");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));

    let rt = tokio::runtime::Runtime::new().unwrap();

    for &max_samples in &[10usize, 100, 500, 1000] {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::new("max_samples", max_samples),
            &max_samples,
            |b, &n| {
                let executor = Arc::new(
                    AdaptiveHedgeExecutor::new(never_hedge_config())
                        .unwrap()
                        .with_max_samples(n)
                        .unwrap(),
                );
                // Pre-warm: fill the ring so every measured call is steady-state.
                rt.block_on(async {
                    for _ in 0..n {
                        let _ = executor.call(|| async { Ok::<u64, &str>(42) }).await;
                    }
                });

                b.to_async(&rt).iter(|| {
                    let executor = Arc::clone(&executor);
                    async move {
                        let result = executor
                            .call(|| async { Ok::<u64, &str>(black_box(42)) })
                            .await;
                        black_box(result)
                    }
                });
            },
        );
    }

    group.finish();
}

// ── Bench 4: Write-lock contention ────────────────────────────────────────────

/// Concurrent callers contending on `AdaptiveHedgeExecutor`'s `RwLock<LatencyTracker>`.
///
/// `call()` takes a write lock at the end to `record()` the observed call latency.
/// Under high concurrency this write lock serialises all callers.
/// Throughput is reported per-element (per individual call) to show the per-task cost.
#[expect(
    clippy::excessive_nesting,
    reason = "task fanout uses nested async closures by design"
)]
fn bench_adaptive_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("hedge/adaptive/contention");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(40); // Fewer samples: each iter spawns N tasks

    for &num_tasks in &[2usize, 8, 32, 128] {
        group.throughput(Throughput::Elements(num_tasks as u64));
        group.bench_with_input(
            BenchmarkId::new("concurrent_calls", num_tasks),
            &num_tasks,
            |b, &n| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let executor = Arc::new(
                    AdaptiveHedgeExecutor::new(never_hedge_config())
                        .unwrap()
                        .with_max_samples(1000)
                        .unwrap(),
                );
                // Pre-warm so histogram updates are part of every measured record().
                rt.block_on(async {
                    for _ in 0..1000 {
                        let _ = executor.call(|| async { Ok::<u64, &str>(42) }).await;
                    }
                });

                b.to_async(&rt).iter(|| {
                    let executor = Arc::clone(&executor);
                    async move {
                        let mut handles = Vec::with_capacity(n);
                        for _ in 0..n {
                            let executor = Arc::clone(&executor);
                            handles.push(tokio::spawn(async move {
                                executor
                                    .call(|| async { Ok::<u64, &str>(black_box(42)) })
                                    .await
                            }));
                        }
                        for handle in handles {
                            let _ = handle.await;
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_call_no_hedge,
    bench_adaptive_overhead,
    bench_adaptive_sample_scaling,
    bench_adaptive_contention,
);
criterion_main!(benches);
