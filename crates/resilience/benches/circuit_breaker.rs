//! Benchmarks for CircuitBreaker hot paths
//!
//! Measures:
//! - `try_acquire` (Closed / HalfOpen gate check)
//! - `record_outcome` (state machine transition)
//! - `call` happy path (acquire → execute → record)
//! - Contention under concurrent callers

use std::{hint::black_box, sync::Arc, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome};

fn closed_config() -> CircuitBreakerConfig {
    CircuitBreakerConfig {
        failure_threshold: 1000,
        reset_timeout: Duration::from_mins(1),
        min_operations: 1000,
        ..Default::default()
    }
}

fn cb_try_acquire(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/try_acquire");

    // Closed state — fast path, should be < 100ns
    group.bench_function("closed", |b| {
        let cb = CircuitBreaker::new(closed_config()).unwrap();
        b.iter(|| {
            let _ = black_box(cb.try_acquire::<&str>());
        });
    });

    // HalfOpen state — checks probe count
    group.bench_function("half_open", |b| {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(1),
            max_half_open_operations: u32::MAX,
            min_operations: 1,
            ..Default::default()
        })
        .unwrap();

        // Trip to Open, then let it transition to HalfOpen
        cb.record_outcome(Outcome::Failure);
        std::thread::sleep(Duration::from_millis(5));
        cb.try_acquire::<&str>().unwrap(); // triggers Open → HalfOpen

        b.iter(|| {
            let _ = black_box(cb.try_acquire::<&str>());
        });
    });

    group.finish();
}

fn cb_record_outcome(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/record_outcome");

    // Success in Closed — the common case
    group.bench_function("success_closed", |b| {
        let cb = CircuitBreaker::new(closed_config()).unwrap();
        b.iter(|| {
            cb.record_outcome(black_box(Outcome::Success));
        });
    });

    // Failure in Closed (no trip — threshold is high)
    group.bench_function("failure_closed_no_trip", |b| {
        let cb = CircuitBreaker::new(closed_config()).unwrap();
        b.iter(|| {
            cb.record_outcome(black_box(Outcome::Failure));
        });
    });

    // With sliding window
    group.bench_function("success_sliding_window_10", |b| {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            sliding_window_size: 10,
            failure_rate_threshold: Some(0.9),
            ..closed_config()
        })
        .unwrap();
        b.iter(|| {
            cb.record_outcome(black_box(Outcome::Success));
        });
    });

    group.finish();
}

fn cb_call_happy_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/call");

    group.bench_function("success", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cb = Arc::new(CircuitBreaker::new(closed_config()).unwrap());

        b.to_async(&rt).iter(|| {
            let cb = Arc::clone(&cb);
            async move {
                let result = cb
                    .call(|| Box::pin(async { Ok::<u64, &str>(black_box(42)) }))
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

#[expect(clippy::excessive_nesting)]
fn cb_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/contention");
    group.sample_size(40);

    for &num_tasks in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("concurrent_call", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let cb = Arc::new(CircuitBreaker::new(closed_config()).unwrap());

                b.to_async(&rt).iter(|| {
                    let cb = Arc::clone(&cb);
                    async move {
                        let mut handles = Vec::with_capacity(num_tasks);
                        for _ in 0..num_tasks {
                            let cb = Arc::clone(&cb);
                            handles.push(tokio::spawn(async move {
                                cb.call(|| Box::pin(async { Ok::<u64, &str>(black_box(42)) }))
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
    cb_try_acquire,
    cb_record_outcome,
    cb_call_happy_path,
    cb_contention,
);
criterion_main!(benches);
