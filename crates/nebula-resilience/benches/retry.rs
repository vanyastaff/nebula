//! Benchmarks for Retry pattern
//!
//! Measures:
//! - Different retry strategies (Fixed, Exponential, Fibonacci)
//! - Jitter calculation overhead
//! - retry() function overhead
//! - Impact of max_attempts on performance

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use nebula_resilience::{retry, RetryStrategy, ResilienceError};
use std::sync::Arc;
use std::time::Duration;

fn retry_strategy_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/strategy_creation");

    group.bench_function("fixed_delay", |b| {
        b.iter(|| {
            black_box(RetryStrategy::fixed_delay(
                3,
                Duration::from_millis(100),
            ))
        });
    });

    group.bench_function("exponential_backoff", |b| {
        b.iter(|| {
            black_box(RetryStrategy::exponential_backoff(
                5,
                Duration::from_millis(100),
            ))
        });
    });

    group.bench_function("linear_backoff", |b| {
        b.iter(|| {
            black_box(RetryStrategy::linear_backoff(
                5,
                Duration::from_millis(100),
            ))
        });
    });

    group.finish();
}

fn retry_delay_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/delay_calculation");

    for strategy_type in &["fixed", "linear", "exponential"] {
        group.bench_function(*strategy_type, |b| {
            let strategy = match *strategy_type {
                "fixed" => RetryStrategy::fixed_delay(
                    3,
                    Duration::from_millis(100),
                ),
                "linear" => RetryStrategy::linear_backoff(
                    3,
                    Duration::from_millis(100),
                ),
                "exponential" => RetryStrategy::exponential_backoff(
                    3,
                    Duration::from_millis(100),
                ),
                _ => unreachable!(),
            };

            b.iter(|| {
                for attempt in 1..=3 {
                    black_box(strategy.delay_for_attempt(attempt));
                }
            });
        });
    }

    group.finish();
}

fn retry_successful_operation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/successful_operation");

    // Benchmark: Successful operation (no retries needed)
    for &max_attempts in &[1, 3, 5, 10] {
        group.bench_with_input(
            BenchmarkId::new("fixed_delay", max_attempts),
            &max_attempts,
            |b, &max_attempts| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let strategy = RetryStrategy::fixed_delay(
                    max_attempts,
                    Duration::from_millis(10),
                );

                b.to_async(&rt).iter(|| async {
                    let result = retry(strategy.clone(), || async {
                        Ok::<_, ResilienceError>(black_box(42))
                    }).await;
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn retry_with_failures(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/with_failures");
    group.sample_size(30); // Reduce sample size since this involves actual retries

    // Benchmark: Operation fails N-1 times, succeeds on last attempt
    for &max_attempts in &[2, 3, 5] {
        group.bench_with_input(
            BenchmarkId::new("fail_until_last", max_attempts),
            &max_attempts,
            |b, &max_attempts| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let strategy = Arc::new(RetryStrategy::fixed_delay(
                    max_attempts,
                    Duration::from_millis(1), // Minimal delay for benchmarking
                ));

                b.to_async(&rt).iter(|| {
                    let strategy = Arc::clone(&strategy);
                    let mut attempt_count = 0;
                    async move {
                        let result = retry((*strategy).clone(), || {
                            attempt_count += 1;
                            async move {
                                if attempt_count < max_attempts {
                                    Err(ResilienceError::custom("fail"))
                                } else {
                                    Ok::<_, ResilienceError>(black_box(42))
                                }
                            }
                        }).await;
                        black_box(result)
                    }
                });
            },
        );
    }

    group.finish();
}

fn retry_exponential_vs_linear(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/backoff_comparison");
    group.sample_size(30);

    // Compare exponential vs fibonacci for same number of retries
    let max_attempts = 5;

    group.bench_function("exponential_backoff", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let strategy = Arc::new(RetryStrategy::exponential_backoff(
            max_attempts,
            Duration::from_millis(1),
        ));

        b.to_async(&rt).iter(|| {
            let strategy = Arc::clone(&strategy);
            let mut attempt_count = 0;
            async move {
                let result = retry((*strategy).clone(), || {
                    attempt_count += 1;
                    async move {
                        if attempt_count < max_attempts {
                            Err(ResilienceError::custom("fail"))
                        } else {
                            Ok::<_, ResilienceError>(42)
                        }
                    }
                }).await;
                black_box(result)
            }
        });
    });

    group.bench_function("linear_backoff", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let strategy = Arc::new(RetryStrategy::linear_backoff(
            max_attempts,
            Duration::from_millis(1),
        ));

        b.to_async(&rt).iter(|| {
            let strategy = Arc::clone(&strategy);
            let mut attempt_count = 0;
            async move {
                let result = retry((*strategy).clone(), || {
                    attempt_count += 1;
                    async move {
                        if attempt_count < max_attempts {
                            Err(ResilienceError::custom("fail"))
                        } else {
                            Ok::<_, ResilienceError>(42)
                        }
                    }
                }).await;
                black_box(result)
            }
        });
    });

    group.finish();
}

fn retry_should_retry_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/should_retry");

    let strategy = RetryStrategy::exponential_backoff(
        5,
        Duration::from_millis(100),
    );

    // Benchmark: Check if error should be retried
    group.bench_function("transient_error", |b| {
        let error = ResilienceError::timeout(Duration::from_secs(1));
        b.iter(|| {
            black_box(strategy.should_retry(&error))
        });
    });

    group.bench_function("permanent_error", |b| {
        let error = ResilienceError::custom("permanent failure");
        b.iter(|| {
            black_box(strategy.should_retry(&error))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    retry_strategy_creation,
    retry_delay_calculation,
    retry_successful_operation,
    retry_with_failures,
    retry_exponential_vs_linear,
    retry_should_retry_check,
);

criterion_main!(benches);
