//! Benchmarks for hedge executor overhead and contention behavior

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::patterns::hedge::{HedgeConfig, HedgeExecutor};
use nebula_resilience::ResilienceError;
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn hedge_execute_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("hedge/execute");

    group.bench_function("primary_fast", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let executor = Arc::new(HedgeExecutor::new(HedgeConfig {
            hedge_delay: Duration::from_millis(25),
            max_hedges: 1,
            exponential_backoff: false,
            backoff_multiplier: 1.0,
        }));

        b.to_async(&rt).iter(|| {
            let executor = Arc::clone(&executor);
            async move {
                let result = executor
                    .execute(|| async {
                        tokio::task::yield_now().await;
                        Ok::<_, ResilienceError>(black_box(42_u64))
                    })
                    .await;
                black_box(result)
            }
        });
    });

    group.bench_function("hedge_wins", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let executor = Arc::new(HedgeExecutor::new(HedgeConfig {
            hedge_delay: Duration::from_millis(2),
            max_hedges: 1,
            exponential_backoff: false,
            backoff_multiplier: 1.0,
        }));

        b.to_async(&rt).iter(|| {
            let executor = Arc::clone(&executor);
            let calls = Arc::new(AtomicUsize::new(0));
            async move {
                let result = executor
                    .execute({
                        let calls = Arc::clone(&calls);
                        move || {
                            let calls = Arc::clone(&calls);
                            async move {
                                let call_index = calls.fetch_add(1, Ordering::SeqCst);
                                if call_index == 0 {
                                    tokio::time::sleep(Duration::from_millis(20)).await;
                                    Ok::<_, ResilienceError>("primary")
                                } else {
                                    tokio::time::sleep(Duration::from_millis(1)).await;
                                    Ok::<_, ResilienceError>("hedge")
                                }
                            }
                        }
                    })
                    .await;

                black_box((result, calls.load(Ordering::SeqCst)))
            }
        });
    });

    group.finish();
}

fn hedge_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("hedge/contention");
    group.sample_size(40);

    for &num_tasks in &[8usize, 32, 64] {
        group.bench_with_input(
            BenchmarkId::new("parallel_fast", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().expect("runtime");
                let executor = Arc::new(HedgeExecutor::new(HedgeConfig {
                    hedge_delay: Duration::from_millis(2),
                    max_hedges: 2,
                    exponential_backoff: true,
                    backoff_multiplier: 2.0,
                }));

                b.to_async(&rt).iter(|| {
                    let executor = Arc::clone(&executor);
                    async move {
                        let mut handles = Vec::with_capacity(num_tasks);
                        for _ in 0..num_tasks {
                            let executor = Arc::clone(&executor);
                            handles.push(tokio::spawn(async move {
                                executor
                                    .execute(|| async {
                                        tokio::task::yield_now().await;
                                        Ok::<_, ResilienceError>(1_u8)
                                    })
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

criterion_group!(benches, hedge_execute_overhead, hedge_contention);
criterion_main!(benches);
