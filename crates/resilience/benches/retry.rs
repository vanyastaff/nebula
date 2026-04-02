//! Benchmarks for the retry pattern.
//!
//! Covers three layers of the hot path:
//!
//! - **`BackoffConfig::delay_for`** — pure delay calculation (no I/O, no allocation).
//!   Fixed, Linear, Exponential, Fibonacci, and Custom strategies, all parametric
//!   over attempt number (0–9) to capture variance across the range.
//! - **Retry loop (immediate success)** — full async round-trip measuring scheduler
//!   overhead with zero retries: acquire config → call → return.
//! - **Retry loop (fail N times then succeed)** — measures classification, backoff
//!   computation, and re-scheduling overhead across 1, 2, and 4 failures.
//!   Uses `BackoffConfig::Fixed(Duration::ZERO)` to isolate logic cost from sleep time.
//! - **Jitter overhead** — comparison of `JitterConfig::None` vs `JitterConfig::Full`
//!   measured through the full retry loop at zero-delay backoff.
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench retry
//! ```

use std::future::Future;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::retry::{BackoffConfig, JitterConfig, RetryConfig};

// ── BackoffConfig::delay_for ──────────────────────────────────────────────────

fn bench_backoff_strategies(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/backoff");

    let fixed = BackoffConfig::Fixed(Duration::from_millis(100));
    group.bench_function("fixed", |b| {
        b.iter(|| {
            for attempt in 0u32..10 {
                black_box(fixed.delay_for(black_box(attempt)));
            }
        });
    });

    let linear = BackoffConfig::Linear {
        base: Duration::from_millis(50),
        max: Duration::from_secs(5),
    };
    group.bench_function("linear", |b| {
        b.iter(|| {
            for attempt in 0u32..10 {
                black_box(linear.delay_for(black_box(attempt)));
            }
        });
    });

    let exp = BackoffConfig::exponential_default();
    group.bench_function("exponential", |b| {
        b.iter(|| {
            for attempt in 0u32..10 {
                black_box(exp.delay_for(black_box(attempt)));
            }
        });
    });

    let fib = BackoffConfig::Fibonacci {
        base: Duration::from_millis(50),
        max: Duration::from_secs(10),
    };
    group.bench_function("fibonacci", |b| {
        b.iter(|| {
            for attempt in 0u32..10 {
                black_box(fib.delay_for(black_box(attempt)));
            }
        });
    });

    // 10 delays → spills past SmallVec<[Duration; 8]> inline capacity intentionally,
    // exercising the heap-spill path alongside the inline path in the comparison.
    let custom = BackoffConfig::Custom(
        (1u32..=10)
            .map(|i| Duration::from_millis(u64::from(i) * 100))
            .collect(),
    );
    group.bench_function("custom", |b| {
        b.iter(|| {
            for attempt in 0u32..10 {
                black_box(custom.delay_for(black_box(attempt)));
            }
        });
    });

    group.finish();
}

// Single attempt at a high attempt index where exponential math is expensive
fn bench_backoff_high_attempt(c: &mut Criterion) {
    let exp = BackoffConfig::exponential_default();
    let mut group = c.benchmark_group("retry/backoff/exponential_attempt");
    for attempt in [0u32, 3, 6, 9, 15] {
        group.bench_with_input(BenchmarkId::from_parameter(attempt), &attempt, |b, &n| {
            b.iter(|| black_box(exp.delay_for(black_box(n))));
        });
    }
    group.finish();
}

// ── Retry loop helpers ────────────────────────────────────────────────────────

/// Operation that always succeeds immediately.
async fn always_ok() -> Result<u64, ()> {
    Ok(black_box(42))
}

/// Factory: returns an operation that fails `n` times, then succeeds.
///
/// Each call constructs a fresh `Arc<AtomicU32>` — this allocation is the reason
/// factory creation belongs in iter_batched setup, not inside iter.
fn fail_n_then_ok(
    n: u32,
) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = Result<u64, ()>> + Send>> {
    let counter = Arc::new(AtomicU32::new(0));
    move || {
        let c = counter.clone();
        Box::pin(async move {
            if c.fetch_add(1, Ordering::Relaxed) < n {
                Err(())
            } else {
                Ok(black_box(42u64))
            }
        })
    }
}

// ── Retry loop ────────────────────────────────────────────────────────────────

fn bench_retry_success_first_attempt(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Config constructed in setup (unmeasured); routine measures only the retry loop.
    c.bench_function("retry/loop/success_first_attempt", |b| {
        b.to_async(&rt).iter_batched(
            || RetryConfig::<()>::new(3).unwrap(),
            |cfg| async move {
                let result =
                    nebula_resilience::retry_with_inner(cfg, || async { always_ok().await }).await;
                black_box(result.unwrap())
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_retry_fail_then_succeed(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("retry/loop/fail_then_succeed");

    for failures in [1u32, 2, 4] {
        group.bench_with_input(
            BenchmarkId::new("failures", failures),
            &failures,
            |b, &n| {
                // iter_batched: config construction + Arc<AtomicU32> alloc in setup,
                // only the retry loop itself is measured.
                b.to_async(&rt).iter_batched(
                    || {
                        let cfg = RetryConfig::<()>::new(n + 1)
                            .unwrap()
                            .backoff(BackoffConfig::Fixed(Duration::ZERO));
                        (cfg, fail_n_then_ok(n))
                    },
                    |(cfg, op)| async move {
                        black_box(nebula_resilience::retry_with_inner(cfg, op).await.unwrap())
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

// ── Jitter overhead ───────────────────────────────────────────────────────────

fn bench_jitter_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("retry/jitter");

    group.bench_function("none", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let cfg = RetryConfig::<()>::new(4)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO));
                (cfg, fail_n_then_ok(3))
            },
            |(cfg, op)| async move {
                black_box(nebula_resilience::retry_with_inner(cfg, op).await)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("full_random", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let cfg = RetryConfig::<()>::new(4)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO))
                    .jitter(JitterConfig::Full { factor: 0.5, seed: None });
                (cfg, fail_n_then_ok(3))
            },
            |(cfg, op)| async move {
                black_box(nebula_resilience::retry_with_inner(cfg, op).await)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("full_seeded", |b| {
        b.to_async(&rt).iter_batched(
            || {
                let cfg = RetryConfig::<()>::new(4)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO))
                    .jitter(JitterConfig::Full { factor: 0.5, seed: Some(0xdead_beef) });
                (cfg, fail_n_then_ok(3))
            },
            |(cfg, op)| async move {
                black_box(nebula_resilience::retry_with_inner(cfg, op).await)
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_backoff_strategies,
    bench_backoff_high_attempt,
    bench_retry_success_first_attempt,
    bench_retry_fail_then_succeed,
    bench_jitter_overhead,
);
criterion_main!(benches);
