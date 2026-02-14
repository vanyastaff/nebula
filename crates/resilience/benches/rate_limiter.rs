//! Benchmarks for Rate Limiter patterns
//!
//! Measures:
//! - TokenBucket throughput
//! - LeakyBucket throughput
//! - SlidingWindow throughput
//! - AdaptiveRateLimiter throughput
//! - GovernorRateLimiter throughput (GCRA)
//! - Comparison between different algorithms

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::{
    AdaptiveRateLimiter, LeakyBucket, RateLimiter, ResilienceError, SlidingWindow, TokenBucket,
};
use std::hint::black_box;
use std::sync::Arc;

fn rate_limiter_acquire(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limiter/acquire");

    // TokenBucket
    for &rate in &[100, 1000, 10000] {
        group.bench_with_input(BenchmarkId::new("token_bucket", rate), &rate, |b, &rate| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let limiter = Arc::new(TokenBucket::new(rate, rate as f64));

            b.to_async(&rt).iter(|| {
                let limiter = Arc::clone(&limiter);
                async move { black_box(limiter.acquire().await) }
            });
        });
    }

    // LeakyBucket
    for &rate in &[100, 1000, 10000] {
        group.bench_with_input(BenchmarkId::new("leaky_bucket", rate), &rate, |b, &rate| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let limiter = Arc::new(LeakyBucket::new(rate, rate as f64));

            b.to_async(&rt).iter(|| {
                let limiter = Arc::clone(&limiter);
                async move { black_box(limiter.acquire().await) }
            });
        });
    }

    // SlidingWindow
    for &rate in &[100, 1000, 10000] {
        group.bench_with_input(
            BenchmarkId::new("sliding_window", rate),
            &rate,
            |b, &rate| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let limiter = Arc::new(SlidingWindow::new(std::time::Duration::from_secs(1), rate));

                b.to_async(&rt).iter(|| {
                    let limiter = Arc::clone(&limiter);
                    async move { black_box(limiter.acquire().await) }
                });
            },
        );
    }

    group.finish();
}

fn rate_limiter_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limiter/execute");

    // TokenBucket execute
    group.bench_function("token_bucket_1000rps", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let limiter = Arc::new(TokenBucket::new(1000, 1000.0));

        b.to_async(&rt).iter(|| {
            let limiter = Arc::clone(&limiter);
            async move {
                let result = limiter
                    .execute(|| async { Ok::<_, ResilienceError>(black_box(42)) })
                    .await;
                black_box(result)
            }
        });
    });

    // AdaptiveRateLimiter execute
    group.bench_function("adaptive_1000rps", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let limiter = Arc::new(AdaptiveRateLimiter::new(
            1000.0,  // initial_rate
            100.0,   // min_rate
            10000.0, // max_rate
        ));

        b.to_async(&rt).iter(|| {
            let limiter = Arc::clone(&limiter);
            async move {
                let result = limiter
                    .execute(|| async { Ok::<_, ResilienceError>(black_box(42)) })
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

fn rate_limiter_current_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limiter/current_rate");

    group.bench_function("token_bucket", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let limiter = TokenBucket::new(1000, 1000.0);

        b.to_async(&rt)
            .iter(|| async { black_box(limiter.current_rate().await) });
    });

    group.bench_function("sliding_window", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let limiter = SlidingWindow::new(std::time::Duration::from_secs(1), 1000);

        b.to_async(&rt)
            .iter(|| async { black_box(limiter.current_rate().await) });
    });

    group.finish();
}

#[expect(clippy::excessive_nesting)]
fn rate_limiter_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("rate_limiter/contention");
    group.sample_size(50); // Reduce sample size for concurrency tests

    // Measure throughput under high contention (multiple concurrent tasks)
    for &num_tasks in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("concurrent_acquire", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let limiter = Arc::new(TokenBucket::new(10000, 10000.0));

                b.to_async(&rt).iter(|| {
                    let limiter = Arc::clone(&limiter);
                    async move {
                        let mut handles = vec![];
                        for _ in 0..num_tasks {
                            let limiter = Arc::clone(&limiter);
                            let handle = tokio::spawn(async move { limiter.acquire().await });
                            handles.push(handle);
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
    rate_limiter_acquire,
    rate_limiter_execute,
    rate_limiter_current_rate,
    rate_limiter_contention,
);

criterion_main!(benches);
