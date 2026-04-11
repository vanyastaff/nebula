//! Benchmarks for Bulkhead (semaphore-based concurrency limiter)
//!
//! Measures:
//! - `acquire` + drop permit (uncontended)
//! - `call` happy path
//! - Contention under concurrent callers

use std::{hint::black_box, sync::Arc};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::bulkhead::{Bulkhead, BulkheadConfig};

fn bulkhead_acquire(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulkhead/acquire");

    for &concurrency in &[10, 100, 1000] {
        group.bench_with_input(
            BenchmarkId::new("uncontended", concurrency),
            &concurrency,
            |b, &concurrency| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let bh = Bulkhead::new(BulkheadConfig {
                    max_concurrency: concurrency,
                    queue_size: 100,
                    timeout: None,
                })
                .unwrap();

                b.to_async(&rt).iter(|| async {
                    let permit = bh.acquire::<&str>().await.unwrap();
                    black_box(permit);
                    // permit dropped here — returns to semaphore
                });
            },
        );
    }

    group.finish();
}

fn bulkhead_call(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulkhead/call");

    group.bench_function("success", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let bh = Bulkhead::new(BulkheadConfig {
            max_concurrency: 100,
            queue_size: 100,
            timeout: None,
        })
        .unwrap();

        b.to_async(&rt).iter(|| async {
            let result = bh
                .call::<u64, &str, _>(|| Box::pin(async { Ok(black_box(42)) }))
                .await;
            black_box(result)
        });
    });

    group.finish();
}

#[expect(clippy::excessive_nesting)]
fn bulkhead_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("bulkhead/contention");
    group.sample_size(40);

    for &num_tasks in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("concurrent_call", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let bh = Arc::new(
                    Bulkhead::new(BulkheadConfig {
                        max_concurrency: num_tasks,
                        queue_size: num_tasks,
                        timeout: None,
                    })
                    .unwrap(),
                );

                b.to_async(&rt).iter(|| {
                    let bh = Arc::clone(&bh);
                    async move {
                        let mut handles = Vec::with_capacity(num_tasks);
                        for _ in 0..num_tasks {
                            let bh = Arc::clone(&bh);
                            handles.push(tokio::spawn(async move {
                                bh.call::<u64, &str, _>(|| Box::pin(async { Ok(black_box(42)) }))
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
    bulkhead_acquire,
    bulkhead_call,
    bulkhead_contention,
);
criterion_main!(benches);
