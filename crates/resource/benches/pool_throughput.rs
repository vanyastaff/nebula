// Pool throughput benchmarks.
//
// Measures raw pool acquire/release overhead with a zero-cost resource
// (no I/O, instant create/recycle/cleanup).

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// -- Minimal no-op resource for benchmarking pool overhead only --

#[derive(Debug, Clone)]
struct NoOpConfig;

impl Config for NoOpConfig {}

struct NoOpResource;

impl Resource for NoOpResource {
    type Config = NoOpConfig;
    type Instance = u64;

    fn id(&self) -> &str {
        "bench-noop"
    }

    async fn create(&self, _config: &NoOpConfig, _ctx: &Context) -> Result<u64> {
        Ok(0)
    }

    async fn is_valid(&self, _instance: &u64) -> Result<bool> {
        Ok(true)
    }

    async fn recycle(&self, _instance: &mut u64) -> Result<()> {
        Ok(())
    }

    async fn cleanup(&self, _instance: u64) -> Result<()> {
        Ok(())
    }
}

fn bench_ctx() -> Context {
    Context::new(Scope::Global, "bench-wf", "bench-ex")
}

fn pool_config(max_size: usize) -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(5),
        idle_timeout: Duration::from_secs(3600),
        max_lifetime: Duration::from_secs(3600),
        validation_interval: Duration::from_secs(3600),
        maintenance_interval: None,
        ..Default::default()
    }
}

fn single_thread_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("failed to build runtime");

    let pool = Pool::new(NoOpResource, NoOpConfig, pool_config(64)).expect("failed to create pool");
    let ctx = bench_ctx();

    // Warm up: acquire and return one instance so subsequent acquires reuse it.
    rt.block_on(async {
        let g = pool.acquire(&ctx).await.unwrap();
        drop(g);
        tokio::time::sleep(Duration::from_millis(10)).await;
    });

    c.bench_function("single_thread_acquire_release", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let ctx = ctx.clone();
            async move {
                let guard = pool.acquire(&ctx).await.unwrap();
                // Simulate minimal use then drop.
                drop(guard);
                // Yield briefly so the spawned return task runs.
                tokio::task::yield_now().await;
                black_box(())
            }
        });
    });
}

fn multi_thread_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("failed to build runtime");

    let pool = Pool::new(NoOpResource, NoOpConfig, pool_config(64)).expect("failed to create pool");
    let ctx = bench_ctx();

    // Warm up pool with some instances.
    rt.block_on(async {
        let mut guards = Vec::new();
        for _ in 0..8 {
            guards.push(pool.acquire(&ctx).await.unwrap());
        }
        drop(guards);
        tokio::time::sleep(Duration::from_millis(20)).await;
    });

    c.bench_function("multi_thread_acquire_release", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let ctx = ctx.clone();
            async move {
                let guard = pool.acquire(&ctx).await.unwrap();
                drop(guard);
                tokio::task::yield_now().await;
                black_box(())
            }
        });
    });
}

fn concurrent_contention(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("failed to build runtime");

    // Small pool to create contention.
    let pool = Pool::new(NoOpResource, NoOpConfig, pool_config(4)).expect("failed to create pool");
    let ctx = bench_ctx();

    c.bench_function("contended_acquire_release_4slots", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let ctx = ctx.clone();
            async move {
                let guard = pool.acquire(&ctx).await.unwrap();
                drop(guard);
                tokio::task::yield_now().await;
                black_box(())
            }
        });
    });
}

criterion_group!(
    benches,
    single_thread_throughput,
    multi_thread_throughput,
    concurrent_contention,
);
criterion_main!(benches);
