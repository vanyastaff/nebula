//! Benchmarks for Retry pattern
//!
//! Measures:
//! - Different retry strategies (Fixed, Exponential, Fibonacci)
//! - Jitter calculation overhead
//! - retry() function overhead
//! - Impact of max_attempts on performance

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use nebula_resilience::{retry, RetryStrategy, ResilienceError};
use std::time::Duration;

fn retry_strategy_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/strategy_creation");

    group.bench_function("fixed_delay", |b| {
        b.iter(|| {
            black_box(RetryStrategy::fixed_delay(
                Duration::from_millis(100),
                3,
            ))
        });
    });

    group.bench_function("exponential_backoff", |b| {
        b.iter(|| {
            black_box(RetryStrategy::exponential_backoff(
                Duration::from_millis(100),
                2.0,
                5,
            ))
        });
    });

    group.bench_function("fibonacci_backoff", |b| {
        b.iter(|| {
            black_box(RetryStrategy::fibonacci_backoff(
                Duration::from_millis(100),
                5,
            ))
        });
    });

    group.finish();
}

fn retry_jitter_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/jitter");

    for jitter_type in &["none", "full", "equal", "decorrelated"] {
        group.bench_function(*jitter_type, |b| {
            let strategy = match *jitter_type {
                "none" => RetryStrategy::exponential_backoff(
                    Duration::from_millis(100),
                    2.0,
                    3,
                ),
                "full" => RetryStrategy::exponential_backoff(
                    Duration::from_millis(100),
                    2.0,
                    3,
                ).with_jitter(0.5),
                "equal" => RetryStrategy::exponential_backoff(
                    Duration::from_millis(100),
                    2.0,
                    3,
                ).with_equal_jitter(),
                "decorrelated" => RetryStrategy::exponential_backoff(
                    Duration::from_millis(100),
                    2.0,
                    3,
                ).with_decorrelated_jitter(),
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
                    Duration::from_millis(10),
                    max_attempts,
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
                let strategy = RetryStrategy::fixed_delay(
                    Duration::from_millis(1), // Minimal delay for benchmarking
                    max_attempts,
                );

                b.to_async(&rt).iter(|| {
                    let mut attempt_count = 0;
                    async move {
                        let result = retry(strategy.clone(), || {
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

fn retry_exponential_vs_fibonacci(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/backoff_comparison");
    group.sample_size(30);

    // Compare exponential vs fibonacci for same number of retries
    let max_attempts = 5;

    group.bench_function("exponential_backoff", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let strategy = RetryStrategy::exponential_backoff(
            Duration::from_millis(1),
            2.0,
            max_attempts,
        );

        b.to_async(&rt).iter(|| {
            let mut attempt_count = 0;
            async move {
                let result = retry(strategy.clone(), || {
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

    group.bench_function("fibonacci_backoff", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let strategy = RetryStrategy::fibonacci_backoff(
            Duration::from_millis(1),
            max_attempts,
        );

        b.to_async(&rt).iter(|| {
            let mut attempt_count = 0;
            async move {
                let result = retry(strategy.clone(), || {
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
        Duration::from_millis(100),
        2.0,
        5,
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
    retry_jitter_calculation,
    retry_successful_operation,
    retry_with_failures,
    retry_exponential_vs_fibonacci,
    retry_should_retry_check,
);

criterion_main!(benches);
