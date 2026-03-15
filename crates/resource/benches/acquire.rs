// Acquire latency and contention benchmarks.
//
// Covers:
// - RSC-T013: acquire latency benchmarks
// - RSC-T014: contention benchmarks under concurrent load

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_core::ResourceKey;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
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

    fn key(&self) -> ResourceKey {
        ResourceKey::try_from("bench-acquire").expect("valid key")
    }

    async fn create(&self, _config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(0)
    }
}

fn ctx() -> Context {
    Context::background(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

fn pool_config(max_size: usize) -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(5),
        maintenance_interval: None,
        ..Default::default()
    }
}

fn acquire_latency_single_thread(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let pool = Pool::new(BenchResource, BenchConfig, pool_config(64)).expect("pool");
    let benchmark_ctx = ctx();

    rt.block_on(async {
        let (guard, _) = pool.acquire(&benchmark_ctx).await.expect("warmup acquire");
        drop(guard);
        tokio::time::sleep(Duration::from_millis(20)).await;
    });

    c.bench_function("acquire_latency_single_thread", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let benchmark_ctx = benchmark_ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&benchmark_ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });
}

fn acquire_latency_multi_thread(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    let pool = Pool::new(BenchResource, BenchConfig, pool_config(64)).expect("pool");
    let benchmark_ctx = ctx();

    c.bench_function("acquire_latency_multi_thread", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let benchmark_ctx = benchmark_ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&benchmark_ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });
}

fn acquire_contention_small_pool(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    let pool = Pool::new(BenchResource, BenchConfig, pool_config(4)).expect("pool");
    let benchmark_ctx = ctx();

    c.bench_function("acquire_contention_pool4", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let benchmark_ctx = benchmark_ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&benchmark_ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });
}

fn acquire_contention_very_small_pool(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    let pool = Pool::new(BenchResource, BenchConfig, pool_config(2)).expect("pool");
    let benchmark_ctx = ctx();

    c.bench_function("acquire_contention_pool2", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let benchmark_ctx = benchmark_ctx.clone();
            async move {
                let (guard, wait) = pool.acquire(&benchmark_ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(wait);
            }
        });
    });
}

criterion_group!(
    benches,
    acquire_latency_single_thread,
    acquire_latency_multi_thread,
    acquire_contention_small_pool,
    acquire_contention_very_small_pool,
);
criterion_main!(benches);
