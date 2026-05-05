//! Benchmarks for shared `PolicyContext` overhead.
//!
//! `PolicyContext` is the public contract that lets Nebula thread cancellation,
//! deadlines, and low-cardinality scope through a composed policy stack. These
//! benchmarks keep that composition layer honest by comparing plain hot paths
//! with their context-aware equivalents.
//!
//! Run with:
//! ```text
//! cargo bench -p nebula-resilience --bench policy_context
//! ```

use std::{hint::black_box, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resilience::{
    PolicyContext, ResiliencePipeline, load_shed, load_shed_with_policy_context, timeout,
    timeout_with_policy_context,
};

fn bench_timeout_context_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let local_timeout = Duration::from_secs(5);
    let empty_context = PolicyContext::empty();
    let deadline_context = PolicyContext::with_timeout(Duration::from_mins(10));
    let mut group = c.benchmark_group("policy_context/timeout");

    group.bench_function("plain_success", |b| {
        b.to_async(&rt).iter(|| async {
            let result = timeout(local_timeout, async { Ok::<u64, &str>(black_box(42)) }).await;
            black_box(result)
        });
    });

    group.bench_function("empty_context_success", |b| {
        b.to_async(&rt).iter(|| async {
            let result = timeout_with_policy_context(&empty_context, local_timeout, async {
                Ok::<u64, &str>(black_box(42))
            })
            .await;
            black_box(result)
        });
    });

    group.bench_function("deadline_context_success", |b| {
        b.to_async(&rt).iter(|| async {
            let result = timeout_with_policy_context(&deadline_context, local_timeout, async {
                Ok::<u64, &str>(black_box(42))
            })
            .await;
            black_box(result)
        });
    });

    group.finish();
}

fn bench_load_shed_context_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let empty_context = PolicyContext::empty();
    let deadline_context = PolicyContext::with_timeout(Duration::from_mins(10));
    let mut group = c.benchmark_group("policy_context/load_shed");

    group.bench_function("plain_pass_through", |b| {
        b.to_async(&rt).iter(|| async {
            let result = load_shed(|| false, || async { Ok::<u64, ()>(black_box(42)) }).await;
            black_box(result)
        });
    });

    group.bench_function("empty_context_pass_through", |b| {
        b.to_async(&rt).iter(|| async {
            let result = load_shed_with_policy_context(
                &empty_context,
                || false,
                || async { Ok::<u64, ()>(black_box(42)) },
            )
            .await;
            black_box(result)
        });
    });

    group.bench_function("deadline_context_pass_through", |b| {
        b.to_async(&rt).iter(|| async {
            let result = load_shed_with_policy_context(
                &deadline_context,
                || false,
                || async { Ok::<u64, ()>(black_box(42)) },
            )
            .await;
            black_box(result)
        });
    });

    group.finish();
}

fn bench_pipeline_context_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let pipeline = ResiliencePipeline::<&str>::builder().build();
    let empty_context = PolicyContext::empty();
    let deadline_context = PolicyContext::with_timeout(Duration::from_mins(10));
    let mut group = c.benchmark_group("policy_context/pipeline");

    group.bench_function("call", |b| {
        b.to_async(&rt).iter(|| {
            let pipeline = &pipeline;
            async move {
                let result = pipeline
                    .call(|| Box::pin(async { Ok::<u64, &str>(black_box(42)) }))
                    .await;
                black_box(result)
            }
        });
    });

    group.bench_function("call_with_empty_context", |b| {
        b.to_async(&rt).iter(|| {
            let pipeline = &pipeline;
            let context = &empty_context;
            async move {
                let result = pipeline
                    .call_with_policy_context(context, || {
                        Box::pin(async { Ok::<u64, &str>(black_box(42)) })
                    })
                    .await;
                black_box(result)
            }
        });
    });

    group.bench_function("call_with_deadline_context", |b| {
        b.to_async(&rt).iter(|| {
            let pipeline = &pipeline;
            let context = &deadline_context;
            async move {
                let result = pipeline
                    .call_with_policy_context(context, || {
                        Box::pin(async { Ok::<u64, &str>(black_box(42)) })
                    })
                    .await;
                black_box(result)
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_timeout_context_overhead,
    bench_load_shed_context_overhead,
    bench_pipeline_context_overhead,
);
criterion_main!(benches);
