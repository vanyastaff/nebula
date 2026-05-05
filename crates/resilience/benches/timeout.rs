//! Benchmarks for timeout pattern overhead and timeout firing latency
//!
//! Measures:
//! - Success-path wrapper overhead for `timeout`
//! - Runtime timer latency and overshoot when timeout expires before a pending future

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resilience::timeout;

fn timeout_wrapper_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("timeout/overhead");

    group.bench_function("direct_yield_once", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");

        b.to_async(&rt).iter(|| async {
            tokio::task::yield_now().await;
            black_box(42_u64)
        });
    });

    group.bench_function("wrapped_yield_once", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let timeout_duration = Duration::from_secs(5);

        b.to_async(&rt).iter(|| async {
            let result = timeout(timeout_duration, async {
                tokio::task::yield_now().await;
                Ok::<_, &str>(black_box(42_u64))
            })
            .await;
            black_box(result)
        });
    });

    group.finish();
}

fn timeout_firing_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("timeout/firing_latency");
    group.measurement_time(Duration::from_secs(12));
    group.sample_size(30);

    group.bench_function("pending_future_1ms", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let timeout_duration = Duration::from_millis(1);

        b.to_async(&rt).iter(|| async {
            let start = Instant::now();
            let result = timeout(
                timeout_duration,
                futures::future::pending::<Result<(), &str>>(),
            )
            .await;
            let elapsed = start.elapsed();
            let overshoot = elapsed.saturating_sub(timeout_duration);
            black_box((result.is_err(), elapsed, overshoot))
        });
    });

    group.bench_function("pending_future_5ms", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let timeout_duration = Duration::from_millis(5);

        b.to_async(&rt).iter(|| async {
            let start = Instant::now();
            let result = timeout(
                timeout_duration,
                futures::future::pending::<Result<(), &str>>(),
            )
            .await;
            let elapsed = start.elapsed();
            let overshoot = elapsed.saturating_sub(timeout_duration);
            black_box((result.is_err(), elapsed, overshoot))
        });
    });

    group.finish();
}

criterion_group!(benches, timeout_wrapper_overhead, timeout_firing_latency);
criterion_main!(benches);
