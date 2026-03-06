//! Benchmarks for Bulkhead pattern
//!
//! Measures:
//! - Acquire fast path
//! - Execute overhead
//! - Concurrent contention behavior
//! - Queue-timeout behavior under saturation

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::{Bulkhead, BulkheadConfig, ResilienceError};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

fn bulkhead_acquire(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulkhead/acquire");

    for &max_concurrency in &[4, 16, 64] {
        group.bench_with_input(
            BenchmarkId::new("fast_path", max_concurrency),
            &max_concurrency,
            |b, &max_concurrency| {
                let rt = tokio::runtime::Runtime::new().expect("runtime");
                let bulkhead = Arc::new(Bulkhead::new(max_concurrency));

                b.to_async(&rt).iter(|| {
                    let bulkhead = Arc::clone(&bulkhead);
                    async move {
                        let permit = bulkhead.acquire().await;
                        black_box(permit)
                    }
                });
            },
        );
    }

    group.finish();
}

fn bulkhead_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulkhead/execute");

    group.bench_function("no_timeout", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let bulkhead = Arc::new(Bulkhead::new(32));

        b.to_async(&rt).iter(|| {
            let bulkhead = Arc::clone(&bulkhead);
            async move {
                let result = bulkhead
                    .execute(|| async { Ok::<_, ResilienceError>(black_box(42_u64)) })
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

#[expect(clippy::excessive_nesting)]
fn bulkhead_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulkhead/contention");
    group.sample_size(50);

    for &num_tasks in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("concurrent_execute", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().expect("runtime");
                let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
                    max_concurrency: 32,
                    queue_size: 1000,
                    timeout: Some(Duration::from_millis(100)),
                }));

                b.to_async(&rt).iter(|| {
                    let bulkhead = Arc::clone(&bulkhead);
                    async move {
                        let mut handles = Vec::with_capacity(num_tasks);
                        for _ in 0..num_tasks {
                            let bulkhead = Arc::clone(&bulkhead);
                            handles.push(tokio::spawn(async move {
                                bulkhead
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

fn bulkhead_queue_timeout(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulkhead/queue_timeout");

    group.bench_function("acquire_timeout_1ms", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
            max_concurrency: 1,
            queue_size: 1,
            timeout: Some(Duration::from_millis(1)),
        }));

        b.to_async(&rt).iter(|| {
            let bulkhead = Arc::clone(&bulkhead);
            async move {
                let permit = bulkhead
                    .acquire()
                    .await
                    .expect("first permit should succeed");
                let timeout_result = bulkhead.acquire().await;
                drop(permit);
                black_box(timeout_result)
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bulkhead_acquire,
    bulkhead_execute,
    bulkhead_contention,
    bulkhead_queue_timeout,
);
criterion_main!(benches);
