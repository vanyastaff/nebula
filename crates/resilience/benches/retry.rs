//! Benchmarks for Retry pattern
//!
//! Measures:
//! - Different retry strategies (Fixed, Exponential, Linear)
//! - Jitter calculation overhead
//! - retry() function overhead
//! - Impact of max_attempts on performance

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::{
    ResilienceError,
    patterns::retry::{
        BackoffPolicy, ConservativeCondition, ExponentialBackoff, FixedDelay, JitterPolicy,
        LinearBackoff, RetryCondition, RetryConfig, RetryStrategy,
    },
};
use std::hint::black_box;
use std::time::Duration;

fn retry_strategy_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/strategy_creation");

    group.bench_function("fixed_delay", |b| {
        b.iter(|| {
            let config = RetryConfig::new(
                FixedDelay::<100>::default(),
                ConservativeCondition::<3>::new(),
            );
            black_box(RetryStrategy::new(config))
        });
    });

    group.bench_function("exponential_backoff", |b| {
        b.iter(|| {
            let config = RetryConfig::new(
                ExponentialBackoff::<100, 20, 5000>::default(),
                ConservativeCondition::<5>::new(),
            );
            black_box(RetryStrategy::new(config))
        });
    });

    group.bench_function("linear_backoff", |b| {
        b.iter(|| {
            let config = RetryConfig::new(
                LinearBackoff::<100, 1000>::default(),
                ConservativeCondition::<5>::new(),
            );
            black_box(RetryStrategy::new(config))
        });
    });

    group.finish();
}

fn retry_delay_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/delay_calculation");

    let fixed_config = RetryConfig::new(
        FixedDelay::<100>::default(),
        ConservativeCondition::<3>::new(),
    );
    let fixed_strategy = RetryStrategy::new(fixed_config).unwrap();

    let exp_config = RetryConfig::new(
        ExponentialBackoff::<100, 20, 5000>::default(),
        ConservativeCondition::<3>::new(),
    );
    let exp_strategy = RetryStrategy::new(exp_config).unwrap();

    let linear_config = RetryConfig::new(
        LinearBackoff::<100, 1000>::default(),
        ConservativeCondition::<3>::new(),
    );
    let linear_strategy = RetryStrategy::new(linear_config).unwrap();

    group.bench_function("fixed", |b| {
        b.iter(|| {
            for attempt in 1..=3 {
                black_box(fixed_strategy.config().backoff.calculate_delay(attempt));
            }
        });
    });

    group.bench_function("exponential", |b| {
        b.iter(|| {
            for attempt in 1..=3 {
                black_box(exp_strategy.config().backoff.calculate_delay(attempt));
            }
        });
    });

    group.bench_function("linear", |b| {
        b.iter(|| {
            for attempt in 1..=3 {
                black_box(linear_strategy.config().backoff.calculate_delay(attempt));
            }
        });
    });

    group.finish();
}

fn retry_successful_operation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/successful_operation");

    // Benchmark: Successful operation (no retries needed)
    for &max_attempts in &[1, 3, 5, 10] {
        group.bench_with_input(
            BenchmarkId::new("fixed_delay", max_attempts),
            &max_attempts,
            |b, &_max_attempts| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let strategy = RetryStrategy::with_policy(
                    FixedDelay::<100>::default(),
                    ConservativeCondition::<3>::new(),
                )
                .unwrap();

                b.to_async(&rt).iter(|| async {
                    let result = strategy
                        .execute(|| async { Ok::<_, ResilienceError>(black_box(42)) })
                        .await;
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

#[expect(clippy::excessive_nesting)]
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
                let strategy = RetryStrategy::with_policy(
                    FixedDelay::<1>::default(),
                    ConservativeCondition::<5>::new(),
                )
                .unwrap();

                b.to_async(&rt).iter(|| {
                    let strategy_ref = &strategy;
                    let mut attempt_count = 0;
                    async move {
                        let result = strategy_ref
                            .execute(|| {
                                attempt_count += 1;
                                let count = attempt_count;
                                async move {
                                    if count < max_attempts {
                                        Err(ResilienceError::custom("fail"))
                                    } else {
                                        Ok::<_, ResilienceError>(black_box(42))
                                    }
                                }
                            })
                            .await;
                        black_box(result)
                    }
                });
            },
        );
    }

    group.finish();
}

#[expect(clippy::excessive_nesting)]
fn retry_backoff_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/backoff_comparison");
    group.sample_size(30);

    // Compare different backoff strategies for same number of retries
    let rt = tokio::runtime::Runtime::new().unwrap();

    group.bench_function("exponential_backoff", |b| {
        let strategy = RetryStrategy::with_policy(
            ExponentialBackoff::<1, 20, 100>::default(), // Small delays for benchmarking
            ConservativeCondition::<5>::new(),
        )
        .unwrap();

        b.to_async(&rt).iter(|| {
            let strategy_ref = &strategy;
            let mut attempt_count = 0;
            async move {
                let result = strategy_ref
                    .execute(|| {
                        attempt_count += 1;
                        let count = attempt_count;
                        async move {
                            if count < 5 {
                                Err(ResilienceError::custom("fail"))
                            } else {
                                Ok::<_, ResilienceError>(42)
                            }
                        }
                    })
                    .await;
                black_box(result)
            }
        });
    });

    group.bench_function("linear_backoff", |b| {
        let strategy = RetryStrategy::with_policy(
            LinearBackoff::<1, 10>::default(), // Small delays for benchmarking
            ConservativeCondition::<5>::new(),
        )
        .unwrap();

        b.to_async(&rt).iter(|| {
            let strategy_ref = &strategy;
            let mut attempt_count = 0;
            async move {
                let result = strategy_ref
                    .execute(|| {
                        attempt_count += 1;
                        let count = attempt_count;
                        async move {
                            if count < 5 {
                                Err(ResilienceError::custom("fail"))
                            } else {
                                Ok::<_, ResilienceError>(42)
                            }
                        }
                    })
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

fn retry_jitter_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/jitter_calculation");

    let base_delay = Duration::from_millis(100);

    group.bench_function("no_jitter", |b| {
        b.iter(|| {
            black_box(JitterPolicy::None.apply(base_delay, None));
        });
    });

    group.bench_function("full_jitter", |b| {
        b.iter(|| {
            black_box(JitterPolicy::Full.apply(base_delay, None));
        });
    });

    group.bench_function("equal_jitter", |b| {
        b.iter(|| {
            black_box(JitterPolicy::Equal.apply(base_delay, None));
        });
    });

    group.bench_function("decorrelated_jitter", |b| {
        b.iter(|| {
            black_box(JitterPolicy::Decorrelated.apply(base_delay, None));
        });
    });

    group.finish();
}

fn retry_error_classification(c: &mut Criterion) {
    let mut group = c.benchmark_group("retry/error_classification");

    let conservative = ConservativeCondition::<5>::new();

    // Benchmark: Check if error should be retried
    group.bench_function("transient_error", |b| {
        let error = ResilienceError::timeout(Duration::from_secs(1));
        b.iter(|| black_box(conservative.should_retry(&error, 1, Duration::ZERO)));
    });

    group.bench_function("terminal_error", |b| {
        let error = ResilienceError::InvalidConfig {
            message: "permanent failure".to_string(),
        };
        b.iter(|| black_box(conservative.should_retry(&error, 1, Duration::ZERO)));
    });

    group.bench_function("custom_retryable_error", |b| {
        let error = ResilienceError::custom("retryable failure");
        b.iter(|| black_box(conservative.should_retry(&error, 1, Duration::ZERO)));
    });

    group.finish();
}

criterion_group!(
    benches,
    retry_strategy_creation,
    retry_delay_calculation,
    retry_successful_operation,
    retry_with_failures,
    retry_backoff_comparison,
    retry_jitter_calculation,
    retry_error_classification,
);

criterion_main!(benches);
