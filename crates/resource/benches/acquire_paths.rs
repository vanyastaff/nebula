// Acquire path micro-benchmarks.
//
// Focuses on hot-path overhead differences:
// - background context (no cancellation select)
// - cancellable context (select-enabled path)
// - backpressure policy dispatch overhead (fail-fast/bounded/adaptive)

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_core::ResourceKey;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::metadata::ResourceMetadata;
use nebula_resource::pool::{AdaptiveBackpressurePolicy, Pool, PoolBackpressurePolicy, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, WorkflowId};

#[derive(Debug, Clone)]
struct BenchConfig;

impl Config for BenchConfig {}

struct BenchResource;

impl Resource for BenchResource {
    type Config = BenchConfig;
    type Instance = u64;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::from_key(ResourceKey::try_from("bench-acquire-paths").expect("valid"))
    }

    async fn create(&self, _config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(0)
    }

    async fn is_reusable(&self, _instance: &Self::Instance) -> Result<bool> {
        Ok(true)
    }

    async fn recycle(&self, _instance: &mut Self::Instance) -> Result<()> {
        Ok(())
    }
}

fn bg_ctx() -> Context {
    Context::background(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

fn cancellable_ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

fn pool_config(max_size: usize, policy: Option<PoolBackpressurePolicy>) -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(5),
        idle_timeout: Duration::from_secs(3600),
        max_lifetime: Duration::from_secs(3600),
        validation_interval: Duration::from_secs(3600),
        maintenance_interval: None,
        backpressure_policy: policy,
        ..Default::default()
    }
}

fn acquire_background_context(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let pool = Pool::new(BenchResource, BenchConfig, pool_config(64, None)).expect("pool creation");
    let ctx = bg_ctx();

    rt.block_on(async {
        let (guard, _) = pool.acquire(&ctx).await.expect("warmup acquire");
        drop(guard);
        tokio::task::yield_now().await;
    });

    c.bench_function("acquire_background_context", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let ctx = ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });
}

fn acquire_cancellable_context(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let pool = Pool::new(BenchResource, BenchConfig, pool_config(64, None)).expect("pool creation");
    let ctx = cancellable_ctx();

    rt.block_on(async {
        let (guard, _) = pool.acquire(&ctx).await.expect("warmup acquire");
        drop(guard);
        tokio::task::yield_now().await;
    });

    c.bench_function("acquire_cancellable_context", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let ctx = ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });
}

fn policy_dispatch_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let ctx = bg_ctx();

    let fail_fast_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config(64, Some(PoolBackpressurePolicy::FailFast)),
    )
    .expect("failfast pool creation");

    let bounded_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config(
            64,
            Some(PoolBackpressurePolicy::BoundedWait {
                timeout: Duration::from_secs(5),
            }),
        ),
    )
    .expect("bounded pool creation");

    let adaptive_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config(
            64,
            Some(PoolBackpressurePolicy::Adaptive(
                AdaptiveBackpressurePolicy::default(),
            )),
        ),
    )
    .expect("adaptive pool creation");

    rt.block_on(async {
        let (g1, _) = fail_fast_pool.acquire(&ctx).await.expect("warmup failfast");
        drop(g1);
        let (g2, _) = bounded_pool.acquire(&ctx).await.expect("warmup bounded");
        drop(g2);
        let (g3, _) = adaptive_pool.acquire(&ctx).await.expect("warmup adaptive");
        drop(g3);
        tokio::task::yield_now().await;
    });

    c.bench_function("acquire_policy_fail_fast", |b| {
        b.to_async(&rt).iter(|| {
            let pool = fail_fast_pool.clone();
            let ctx = ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });

    c.bench_function("acquire_policy_bounded_wait", |b| {
        b.to_async(&rt).iter(|| {
            let pool = bounded_pool.clone();
            let ctx = ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });

    c.bench_function("acquire_policy_adaptive", |b| {
        b.to_async(&rt).iter(|| {
            let pool = adaptive_pool.clone();
            let ctx = ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });
}

criterion_group!(
    benches,
    acquire_background_context,
    acquire_cancellable_context,
    policy_dispatch_overhead,
);
criterion_main!(benches);
