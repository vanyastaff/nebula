//! Benchmarks for layer composition overhead in deep chains
//!
//! Measures:
//! - chain build cost by depth
//! - chain execute overhead with no-op layers
#![expect(
    clippy::excessive_nesting,
    reason = "Composition benchmark scenarios require deeply nested async setup and execution"
)]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::{
    BoxedOperation, LayerBuilder, LayerStack, ResilienceError, ResilienceLayer, RetryableOperation,
};
use std::future::Future;
use std::hint::black_box;
use std::pin::Pin;
use std::sync::Arc;

struct NoopLayer;

impl ResilienceLayer<u64> for NoopLayer {
    fn apply<'a>(
        &'a self,
        operation: &'a BoxedOperation<u64>,
        next: &'a (dyn LayerStack<u64> + Send + Sync),
        _cancellation: Option<&'a nebula_resilience::core::CancellationContext>,
    ) -> Pin<Box<dyn Future<Output = Result<u64, ResilienceError>> + Send + 'a>> {
        next.execute(operation)
    }

    fn name(&self) -> &'static str {
        "noop"
    }
}

fn build_chain(depth: usize) -> Arc<dyn LayerStack<u64> + Send + Sync> {
    let mut builder = LayerBuilder::<u64>::new();
    for _ in 0..depth {
        builder = builder.with_layer(Arc::new(NoopLayer));
    }
    builder.build()
}

fn compose_build_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("compose/build");

    for &depth in &[1, 3, 5, 8, 12, 16] {
        group.bench_with_input(BenchmarkId::new("depth", depth), &depth, |b, &depth| {
            b.iter(|| {
                let chain = build_chain(depth);
                black_box(chain)
            });
        });
    }

    group.finish();
}

fn compose_execute_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("compose/execute");

    for &depth in &[1, 3, 5, 8, 12, 16] {
        group.bench_with_input(BenchmarkId::new("depth", depth), &depth, |b, &depth| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let chain = build_chain(depth);

            b.to_async(&rt).iter(|| {
                let chain = Arc::clone(&chain);
                async move {
                    let operation = || async { Ok::<u64, ResilienceError>(black_box(42)) };
                    let boxed = BoxedOperation::new(operation);
                    let result = chain.execute(&boxed).await;
                    black_box(result)
                }
            });
        });
    }

    group.finish();
}

fn compose_execute_retry_shape(c: &mut Criterion) {
    let mut group = c.benchmark_group("compose/execute_retryable_clone");

    for &depth in &[3, 8, 16] {
        group.bench_with_input(BenchmarkId::new("depth", depth), &depth, |b, &depth| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let chain = build_chain(depth);
            let operation = Arc::new(|| async { Ok::<u64, ResilienceError>(black_box(7)) });

            b.to_async(&rt).iter(|| {
                let chain = Arc::clone(&chain);
                let operation = Arc::clone(&operation);
                async move {
                    let boxed = BoxedOperation::from_arc(
                        operation as Arc<dyn RetryableOperation<u64> + Send + Sync>,
                    );
                    let result = chain.execute(&boxed).await;
                    black_box(result)
                }
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    compose_build_overhead,
    compose_execute_overhead,
    compose_execute_retry_shape,
);
criterion_main!(benches);
