// Contended acquire benchmarks.
//
// Measures pool behavior under concurrent pressure with a small max_size.

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
        ResourceMetadata::from_key(ResourceKey::try_from("bench-acquire-contended").expect("valid"))
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

fn pool_config(max_size: usize) -> PoolConfig {
    pool_config_with_policy(max_size, None)
}

fn pool_config_with_policy(max_size: usize, policy: Option<PoolBackpressurePolicy>) -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(5),
        maintenance_interval: None,
        backpressure_policy: policy,
        ..Default::default()
    }
}

async fn run_contended_round(
    pool: Pool<BenchResource>,
    ctx: Context,
    workers: usize,
    iters: usize,
) {
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let pool = pool.clone();
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            let mut observed_wait_nanos = 0u64;
            let mut successes = 0u64;
            let mut failures = 0u64;
            for _ in 0..iters {
                match pool.acquire(&ctx).await {
                    Ok((guard, wait)) => {
                        observed_wait_nanos =
                            observed_wait_nanos.saturating_add(wait.as_nanos() as u64);
                        successes = successes.saturating_add(1);
                        drop(guard);
                    }
                    Err(_) => {
                        failures = failures.saturating_add(1);
                    }
                }
                tokio::task::yield_now().await;
            }
            (observed_wait_nanos, successes, failures)
        }));
    }

    let mut total_wait = 0u64;
    let mut total_successes = 0u64;
    let mut total_failures = 0u64;
    for h in handles {
        let (wait, successes, failures) = h.await.expect("join");
        total_wait = total_wait.saturating_add(wait);
        total_successes = total_successes.saturating_add(successes);
        total_failures = total_failures.saturating_add(failures);
    }
    black_box((total_wait, total_successes, total_failures));
}

fn contended_8_workers(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");

    let pool = Pool::new(BenchResource, BenchConfig, pool_config(4)).expect("pool");
    let ctx = bg_ctx();

    c.bench_function("contended_acquire_workers8_pool4", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 8, 32).await;
            }
        });
    });
}

fn contended_32_workers(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");

    let pool = Pool::new(BenchResource, BenchConfig, pool_config(4)).expect("pool");
    let ctx = bg_ctx();

    c.bench_function("contended_acquire_workers32_pool4", |b| {
        b.to_async(&rt).iter(|| {
            let pool = pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 32, 16).await;
            }
        });
    });
}

fn contended_policy_compare(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");

    let ctx = bg_ctx();

    let fail_fast_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config_with_policy(4, Some(PoolBackpressurePolicy::FailFast)),
    )
    .expect("pool");

    let bounded_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config_with_policy(
            4,
            Some(PoolBackpressurePolicy::BoundedWait {
                timeout: Duration::from_millis(2),
            }),
        ),
    )
    .expect("pool");

    let adaptive_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config_with_policy(
            4,
            Some(PoolBackpressurePolicy::Adaptive(
                AdaptiveBackpressurePolicy {
                    high_pressure_utilization: 0.8,
                    high_pressure_waiters: 8,
                    low_pressure_timeout: Duration::from_millis(20),
                    high_pressure_timeout: Duration::from_millis(2),
                },
            )),
        ),
    )
    .expect("pool");

    c.bench_function("contended_policy_fail_fast_workers64_pool4", |b| {
        b.to_async(&rt).iter(|| {
            let pool = fail_fast_pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 64, 8).await;
            }
        });
    });

    c.bench_function("contended_policy_bounded_wait_workers64_pool4", |b| {
        b.to_async(&rt).iter(|| {
            let pool = bounded_pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 64, 8).await;
            }
        });
    });

    c.bench_function("contended_policy_adaptive_workers64_pool4", |b| {
        b.to_async(&rt).iter(|| {
            let pool = adaptive_pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 64, 8).await;
            }
        });
    });
}

fn contended_policy_compare_pool8(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("runtime");

    let ctx = bg_ctx();

    let fail_fast_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config_with_policy(8, Some(PoolBackpressurePolicy::FailFast)),
    )
    .expect("pool");

    let bounded_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config_with_policy(
            8,
            Some(PoolBackpressurePolicy::BoundedWait {
                timeout: Duration::from_millis(2),
            }),
        ),
    )
    .expect("pool");

    let adaptive_pool = Pool::new(
        BenchResource,
        BenchConfig,
        pool_config_with_policy(
            8,
            Some(PoolBackpressurePolicy::Adaptive(
                AdaptiveBackpressurePolicy {
                    high_pressure_utilization: 0.8,
                    high_pressure_waiters: 8,
                    low_pressure_timeout: Duration::from_millis(20),
                    high_pressure_timeout: Duration::from_millis(2),
                },
            )),
        ),
    )
    .expect("pool");

    c.bench_function("contended_policy_fail_fast_workers64_pool8", |b| {
        b.to_async(&rt).iter(|| {
            let pool = fail_fast_pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 64, 8).await;
            }
        });
    });

    c.bench_function("contended_policy_bounded_wait_workers64_pool8", |b| {
        b.to_async(&rt).iter(|| {
            let pool = bounded_pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 64, 8).await;
            }
        });
    });

    c.bench_function("contended_policy_adaptive_workers64_pool8", |b| {
        b.to_async(&rt).iter(|| {
            let pool = adaptive_pool.clone();
            let ctx = ctx.clone();
            async move {
                run_contended_round(pool, ctx, 64, 8).await;
            }
        });
    });
}

criterion_group!(
    benches,
    contended_8_workers,
    contended_32_workers,
    contended_policy_compare,
    contended_policy_compare_pool8,
);
criterion_main!(benches);
