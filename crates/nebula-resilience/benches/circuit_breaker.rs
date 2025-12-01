//! Benchmarks for CircuitBreaker pattern
//!
//! Measures:
//! - State transition overhead (Closed -> Open -> HalfOpen -> Closed)
//! - can_execute() check performance
//! - execute() with successful operations
//! - execute() with failures triggering circuit open

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use nebula_resilience::{CircuitBreaker, CircuitBreakerConfig, ResilienceError};
use std::time::Duration;

fn circuit_breaker_closed_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/closed");

    for &threshold in &[5, 10, 50] {
        group.bench_with_input(
            BenchmarkId::new("execute_success", threshold),
            &threshold,
            |b, &_threshold| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let config = CircuitBreakerConfig::<5, 60000>::new().with_min_operations(1);
                let cb = CircuitBreaker::new(config).unwrap();

                b.to_async(&rt).iter(|| async {
                    let result = cb
                        .execute(|| async { Ok::<_, ResilienceError>(black_box(42)) })
                        .await;
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn circuit_breaker_can_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/can_execute");

    // Benchmark: Check if can execute (circuit closed)
    group.bench_function("closed", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = CircuitBreakerConfig::<5, 60000>::new().with_min_operations(1);
        let cb = CircuitBreaker::new(config).unwrap();

        b.to_async(&rt)
            .iter(|| async { black_box(cb.can_execute().await) });
    });

    // Benchmark: Check if can execute (circuit open)
    group.bench_function("open", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = CircuitBreakerConfig::<2, 60000>::new().with_min_operations(1);
        let cb = CircuitBreaker::new(config).unwrap();

        // Trigger circuit open
        rt.block_on(async {
            // Execute minimum operations first
            for _ in 0..2 {
                let _ = cb.execute(|| async { Ok::<(), _>(()) }).await;
            }
            // Now trigger failures
            for _ in 0..3 {
                let _ = cb
                    .execute(|| async { Err::<(), _>(ResilienceError::custom("fail")) })
                    .await;
            }
        });

        b.to_async(&rt)
            .iter(|| async { black_box(cb.can_execute().await) });
    });

    group.finish();
}

fn circuit_breaker_state_transitions(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/transitions");

    // Benchmark: Closed -> Open transition
    group.bench_function("closed_to_open", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();

        b.iter_batched(
            || {
                let config = CircuitBreakerConfig::<3, 60000>::new().with_min_operations(1);
                CircuitBreaker::new(config).unwrap()
            },
            |cb| {
                rt.block_on(async {
                    // Execute minimum operations first
                    for _ in 0..2 {
                        let _ = cb.execute(|| async { Ok::<(), _>(()) }).await;
                    }
                    // Trigger failures to open circuit
                    for _ in 0..4 {
                        let _ = cb
                            .execute(|| async { Err::<(), _>(ResilienceError::custom("fail")) })
                            .await;
                    }
                })
            },
            criterion::BatchSize::SmallInput,
        );
    });

    // Benchmark: Half-Open -> Closed transition
    group.bench_function("halfopen_to_closed", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();

        b.iter_batched(
            || {
                let config = CircuitBreakerConfig::<1, 10>::new().with_min_operations(1);
                let cb = CircuitBreaker::new(config).unwrap();

                // Open the circuit
                rt.block_on(async {
                    // Execute minimum operations first
                    let _ = cb.execute(|| async { Ok::<(), _>(()) }).await;

                    // Trigger failure to open circuit
                    for _ in 0..2 {
                        let _ = cb
                            .execute(|| async { Err::<(), _>(ResilienceError::custom("fail")) })
                            .await;
                    }

                    // Wait for reset timeout
                    tokio::time::sleep(Duration::from_millis(15)).await;
                });

                cb
            },
            |cb| {
                rt.block_on(async {
                    // Successful operation should transition to Closed
                    let _ = cb.execute(|| async { Ok::<_, ResilienceError>(42) }).await;
                })
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn circuit_breaker_stats(c: &mut Criterion) {
    let mut group = c.benchmark_group("circuit_breaker/stats");

    group.bench_function("stats_collection", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let config = CircuitBreakerConfig::<5, 60000>::new().with_min_operations(1);
        let cb = CircuitBreaker::new(config).unwrap();

        b.to_async(&rt)
            .iter(|| async { black_box(cb.stats().await) });
    });

    group.finish();
}

criterion_group!(
    benches,
    circuit_breaker_closed_execute,
    circuit_breaker_can_execute,
    circuit_breaker_state_transitions,
    circuit_breaker_stats,
);

criterion_main!(benches);
