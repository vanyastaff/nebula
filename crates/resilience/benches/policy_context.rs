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

use std::{borrow::Cow, hint::black_box, time::Duration};

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resilience::{
    CallErrorKind, PipelineOutcome, PolicyContext, PolicyScope, ResilienceEvent,
    ResiliencePipeline, load_shed, load_shed_with_policy_context, timeout,
    timeout_with_policy_context,
};

#[derive(Clone)]
struct LegacyCowScope {
    tenant_id: Option<Cow<'static, str>>,
    workflow_id: Option<Cow<'static, str>>,
    action_id: Option<Cow<'static, str>>,
    resource_id: Option<Cow<'static, str>>,
    operation: Option<Cow<'static, str>>,
}

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

fn bench_scope_clone(c: &mut Criterion) {
    let scope = PolicyScope::empty()
        .tenant_id(String::from("tenant-a"))
        .workflow_id(String::from("workflow-a"))
        .action_id(String::from("action-a"))
        .resource_id(String::from("resource-a"))
        .operation(String::from("gmail.poll"));
    let event = ResilienceEvent::PipelineCompleted {
        scope: scope.clone(),
        outcome: PipelineOutcome::Failure {
            error: CallErrorKind::Timeout,
        },
    };
    let legacy = LegacyCowScope {
        tenant_id: Some(Cow::Owned(String::from("tenant-a"))),
        workflow_id: Some(Cow::Owned(String::from("workflow-a"))),
        action_id: Some(Cow::Owned(String::from("action-a"))),
        resource_id: Some(Cow::Owned(String::from("resource-a"))),
        operation: Some(Cow::Owned(String::from("gmail.poll"))),
    };

    let mut group = c.benchmark_group("policy_context/scope");

    group.bench_function("clone_scope_value", |b| {
        b.iter(|| black_box(scope.clone()));
    });

    group.bench_function("clone_pipeline_completed_event", |b| {
        b.iter(|| black_box(event.clone()));
    });

    group.bench_function("legacy_cow_owned_clone", |b| {
        b.iter(|| {
            let cloned = black_box(legacy.clone());
            black_box((
                cloned.tenant_id,
                cloned.workflow_id,
                cloned.action_id,
                cloned.resource_id,
                cloned.operation,
            ))
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_timeout_context_overhead,
    bench_load_shed_context_overhead,
    bench_pipeline_context_overhead,
    bench_scope_clone,
);
criterion_main!(benches);
