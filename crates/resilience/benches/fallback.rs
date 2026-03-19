//! Benchmarks for fallback execution overhead under normal and contention paths
#![expect(
    clippy::excessive_nesting,
    reason = "Benchmark task fanout uses nested async closures and join loops by design"
)]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::ResilienceError;
use nebula_resilience::fallback::{
    ChainFallback, FallbackOperation, FallbackStrategy, ValueFallback,
};
use std::hint::black_box;
use std::sync::Arc;

fn fallback_execute_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("fallback/execute");

    group.bench_function("value_success_path", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let fallback = Arc::new(ValueFallback::new("fallback".to_string()));
        let operation = Arc::new(FallbackOperation::new(fallback));

        b.to_async(&rt).iter(|| {
            let operation = Arc::clone(&operation);
            async move {
                let result = operation
                    .execute(|| async { Ok::<_, ResilienceError>("primary".to_string()) })
                    .await;
                black_box(result)
            }
        });
    });

    group.bench_function("value_error_path", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let fallback = Arc::new(ValueFallback::new("fallback".to_string()));
        let operation = Arc::new(FallbackOperation::new(fallback));

        b.to_async(&rt).iter(|| {
            let operation = Arc::clone(&operation);
            async move {
                let result = operation
                    .execute(|| async { Err::<String, _>(ResilienceError::custom("boom")) })
                    .await;
                black_box(result)
            }
        });
    });

    group.bench_function("chain_error_path", |b| {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let chain = Arc::new(
            ChainFallback::new().add(Arc::new(ValueFallback::new("chain-fallback".to_string()))
                as Arc<dyn FallbackStrategy<String>>),
        );
        let operation = Arc::new(FallbackOperation::new(chain));

        b.to_async(&rt).iter(|| {
            let operation = Arc::clone(&operation);
            async move {
                let result = operation
                    .execute(|| async { Err::<String, _>(ResilienceError::custom("boom")) })
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

fn fallback_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("fallback/contention");
    group.sample_size(40);

    for &num_tasks in &[8usize, 32, 96] {
        group.bench_with_input(
            BenchmarkId::new("error_path_parallel", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().expect("runtime");
                let fallback = Arc::new(ValueFallback::new("fallback".to_string()));
                let operation = Arc::new(FallbackOperation::new(fallback));

                b.to_async(&rt).iter(|| {
                    let operation = Arc::clone(&operation);
                    async move {
                        let mut handles = Vec::with_capacity(num_tasks);
                        for _ in 0..num_tasks {
                            let operation = Arc::clone(&operation);
                            handles.push(tokio::spawn(async move {
                                operation
                                    .execute(|| async {
                                        Err::<String, _>(ResilienceError::custom("boom"))
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

criterion_group!(benches, fallback_execute_overhead, fallback_contention);
criterion_main!(benches);
