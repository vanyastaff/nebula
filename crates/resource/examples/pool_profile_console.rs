//! Tokio-console profiling harness for pool contention.
//!
//! Run with:
//! RUSTFLAGS="--cfg tokio_unstable" cargo run -p nebula-resource --example pool_profile_console

use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolAcquire, PoolConfig, PoolLifetime, PoolSizing};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, WorkflowId};

#[derive(Debug, Clone)]
struct ProfileConfig;

impl Config for ProfileConfig {}

struct ProfileResource;

impl Resource for ProfileResource {
    type Config = ProfileConfig;
    type Instance = u64;

    fn key(&self) -> ResourceKey {
        resource_key!("profile-pool")
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

fn try_init_console_subscriber() -> bool {
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let init_ok = std::panic::catch_unwind(console_subscriber::init).is_ok();
    std::panic::set_hook(old_hook);
    init_ok
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let console_enabled = try_init_console_subscriber();
    if !console_enabled {
        eprintln!(
            "tokio-console disabled: rebuild with RUSTFLAGS=\"--cfg tokio_unstable\" to enable task tracing"
        );
    }

    let pool = Arc::new(
        Pool::new(
            ProfileResource,
            ProfileConfig,
            PoolConfig {
                sizing: PoolSizing {
                    min_size: 0,
                    max_size: 4,
                },
                lifetime: PoolLifetime {
                    maintenance_interval: None,
                    ..Default::default()
                },
                acquire: PoolAcquire {
                    timeout: Duration::from_secs(2),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .expect("pool"),
    );

    let ctx = Context::background(Scope::Global, WorkflowId::new(), ExecutionId::new());

    let workers = 32usize;
    let runtime = Duration::from_secs(20);
    let started = Instant::now();

    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let pool = Arc::clone(&pool);
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            let mut ops = 0u64;
            while started.elapsed() < runtime {
                let (guard, _) = pool.acquire(&ctx).await.expect("acquire");
                drop(guard);
                ops = ops.saturating_add(1);
                tokio::task::yield_now().await;
            }
            ops
        }));
    }

    let mut total_ops = 0u64;
    for h in handles {
        total_ops = total_ops.saturating_add(h.await.expect("join"));
    }

    let stats = pool.stats();
    println!("ops={total_ops}");
    println!("acquired_total={}", stats.total_acquisitions);
    println!("active={} idle={}", stats.active, stats.idle);
    if let Some(lat) = stats.acquire_latency {
        println!(
            "p50={}ms p95={}ms p99={}ms p999={}ms mean={:.2}ms",
            lat.p50_ms, lat.p95_ms, lat.p99_ms, lat.p999_ms, lat.mean_ms
        );
    } else {
        println!("latency=none");
    }
}
