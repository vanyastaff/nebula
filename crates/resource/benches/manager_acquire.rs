// Manager-level acquire benchmarks.
//
// Measures the overhead that Manager::acquire adds on top of Pool::acquire:
// quarantine check, health state lookup, scope compatibility check, and
// before/after hook dispatch.
//
// Comparisons (all in the "manager_vs_pool" benchmark group):
// - pool_direct:              raw Pool::acquire (no manager overhead, baseline)
// - manager_acquire:          Manager::acquire through the full stack
// - manager_acquire_typed:    Manager::acquire_typed (typed downcast path)
// - manager_acquire_mt:       Manager::acquire under a 4-thread runtime

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{
    BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolAcquire, PoolConfig, PoolLifetime, PoolSizing};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, Manager, WorkflowId};

// ---------------------------------------------------------------------------
// Minimal no-op resource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BenchConfig;

impl Config for BenchConfig {}

struct BenchResource;

impl Resource for BenchResource {
    type Config = BenchConfig;
    type Instance = u64;

    fn key(&self) -> ResourceKey {
        resource_key!("bench-manager")
    }

    async fn create(&self, _config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(0)
    }

    async fn is_reusable(
        &self,
        _instance: &Self::Instance,
        _meta: &nebula_resource::pool::InstanceMetadata,
    ) -> Result<bool> {
        Ok(true)
    }

    async fn recycle(
        &self,
        _instance: &mut Self::Instance,
        _meta: &nebula_resource::pool::InstanceMetadata,
    ) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn bg_ctx() -> Context {
    Context::background(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        sizing: PoolSizing {
            min_size: 1,
            max_size: 64,
        },
        acquire: PoolAcquire {
            timeout: Duration::from_secs(5),
            ..Default::default()
        },
        lifetime: PoolLifetime {
            maintenance_interval: None,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn resource_key() -> ResourceKey {
    resource_key!("bench-manager")
}

// ---------------------------------------------------------------------------
// pool_direct: baseline — bypass the manager entirely
// ---------------------------------------------------------------------------

fn pool_direct(group: &mut BenchmarkGroup<WallTime>) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let pool = Pool::new(BenchResource, BenchConfig, pool_config()).expect("pool");
    let ctx = bg_ctx();

    rt.block_on(async {
        let (g, _) = pool.acquire(&ctx).await.expect("warmup");
        drop(g);
        tokio::task::yield_now().await;
    });

    group.bench_function("pool_direct", |b| {
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

// ---------------------------------------------------------------------------
// manager_acquire: Manager::acquire with quarantine/health/scope/hooks
// ---------------------------------------------------------------------------

fn manager_acquire(group: &mut BenchmarkGroup<WallTime>) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let manager = Arc::new(Manager::new());
    let ctx = bg_ctx();
    let key = resource_key();

    manager
        .register_scoped(BenchResource, BenchConfig, pool_config(), Scope::Global)
        .expect("register");

    rt.block_on(async {
        let g = manager.acquire(&key, &ctx).await.expect("warmup");
        drop(g);
        tokio::task::yield_now().await;
    });

    group.bench_function("manager_acquire", |b| {
        b.to_async(&rt).iter(|| {
            let manager = Arc::clone(&manager);
            let ctx = ctx.clone();
            let key = key.clone();
            async move {
                let guard = manager.acquire(&key, &ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(());
            }
        });
    });
}

// ---------------------------------------------------------------------------
// manager_acquire_typed: Manager::acquire_typed — no string key, typed downcast
// ---------------------------------------------------------------------------

fn manager_acquire_typed(group: &mut BenchmarkGroup<WallTime>) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let manager = Arc::new(Manager::new());
    let ctx = bg_ctx();

    manager
        .register_scoped(BenchResource, BenchConfig, pool_config(), Scope::Global)
        .expect("register");

    rt.block_on(async {
        let g = manager
            .acquire_typed::<BenchResource>(&ctx)
            .await
            .expect("warmup");
        drop(g);
        tokio::task::yield_now().await;
    });

    group.bench_function("manager_acquire_typed", |b| {
        b.to_async(&rt).iter(|| {
            let manager = Arc::clone(&manager);
            let ctx = ctx.clone();
            async move {
                let guard = manager
                    .acquire_typed::<BenchResource>(&ctx)
                    .await
                    .expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(());
            }
        });
    });
}

// ---------------------------------------------------------------------------
// manager_acquire_mt: Manager::acquire under multi-threaded runtime
// ---------------------------------------------------------------------------

fn manager_acquire_mt(group: &mut BenchmarkGroup<WallTime>) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");
    let manager = Arc::new(Manager::new());
    let ctx = bg_ctx();
    let key = resource_key();

    manager
        .register_scoped(BenchResource, BenchConfig, pool_config(), Scope::Global)
        .expect("register");

    rt.block_on(async {
        let g = manager.acquire(&key, &ctx).await.expect("warmup");
        drop(g);
        tokio::task::yield_now().await;
    });

    group.bench_function("manager_acquire_mt", |b| {
        b.to_async(&rt).iter(|| {
            let manager = Arc::clone(&manager);
            let ctx = ctx.clone();
            let key = key.clone();
            async move {
                let guard = manager.acquire(&key, &ctx).await.expect("acquire");
                drop(guard);
                tokio::task::yield_now().await;
                black_box(());
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Criterion entry point
// ---------------------------------------------------------------------------

fn manager_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("manager_vs_pool");
    pool_direct(&mut group);
    manager_acquire(&mut group);
    manager_acquire_typed(&mut group);
    manager_acquire_mt(&mut group);
    group.finish();
}

criterion_group!(benches, manager_overhead);
criterion_main!(benches);
