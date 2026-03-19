//! Benchmarks for `ResiliencePipeline` composition and execution overhead.
//!
//! Measures:
//! - pipeline build cost by step count
//! - pipeline execute overhead for the happy path
//! - pipeline retry path overhead

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use nebula_resilience::{
    ResiliencePipeline,
    patterns::{
        bulkhead::{Bulkhead, BulkheadConfig},
        circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
        retry::{BackoffConfig, RetryConfig},
    },
};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

fn build_pipeline_1step() -> ResiliencePipeline<&'static str> {
    ResiliencePipeline::builder()
        .timeout(Duration::from_secs(5))
        .build()
}

fn build_pipeline_2step() -> ResiliencePipeline<&'static str> {
    ResiliencePipeline::builder()
        .timeout(Duration::from_secs(5))
        .retry(RetryConfig::new(3).unwrap().backoff(BackoffConfig::Fixed(Duration::from_millis(1))))
        .build()
}

fn build_pipeline_4step() -> ResiliencePipeline<&'static str> {
    let cb = Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default()).unwrap());
    let bh = Arc::new(Bulkhead::new(BulkheadConfig { max_concurrency: 64, ..Default::default() }).unwrap());
    ResiliencePipeline::builder()
        .timeout(Duration::from_secs(5))
        .retry(RetryConfig::new(3).unwrap().backoff(BackoffConfig::Fixed(Duration::from_millis(1))))
        .circuit_breaker(cb)
        .bulkhead(bh)
        .build()
}

fn pipeline_build_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline/build");

    group.bench_function("1step", |b| b.iter(|| black_box(build_pipeline_1step())));
    group.bench_function("2step", |b| b.iter(|| black_box(build_pipeline_2step())));
    group.bench_function("4step", |b| b.iter(|| black_box(build_pipeline_4step())));

    group.finish();
}

fn pipeline_execute_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline/execute");

    for steps in [1usize, 2, 4] {
        let pipeline: ResiliencePipeline<&str> = match steps {
            1 => build_pipeline_1step(),
            2 => build_pipeline_2step(),
            _ => build_pipeline_4step(),
        };

        group.bench_with_input(BenchmarkId::new("steps", steps), &steps, |b, _| {
            let rt = tokio::runtime::Runtime::new().unwrap();
            b.to_async(&rt).iter(|| {
                let p = &pipeline;
                async move {
                    let result = p.call(|| Box::pin(async { Ok::<u64, &str>(black_box(42)) })).await;
                    black_box(result)
                }
            });
        });
    }

    group.finish();
}

fn pipeline_retry_path(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let pipeline = ResiliencePipeline::<&str>::builder()
        .retry(RetryConfig::new(3).unwrap().backoff(BackoffConfig::Fixed(Duration::ZERO)))
        .build();

    c.bench_function("pipeline/retry_success_first_attempt", |b| {
        b.to_async(&rt).iter(|| {
            let p = &pipeline;
            async move {
                let result = p.call(|| Box::pin(async { Ok::<u64, &str>(black_box(1)) })).await;
                black_box(result)
            }
        });
    });
}

criterion_group!(
    benches,
    pipeline_build_overhead,
    pipeline_execute_overhead,
    pipeline_retry_path,
);
criterion_main!(benches);
