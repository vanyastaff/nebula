//! Policy quality probe under contention.
//!
//! Prints success/failure counts and mean wait for each backpressure policy.
//! Useful as a quick service-quality companion to Criterion timings.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{
    AdaptiveBackpressurePolicy, Pool, PoolAcquire, PoolBackpressurePolicy, PoolConfig,
    PoolLifetime, PoolSizing,
};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, WorkflowId};

#[derive(Debug, Clone)]
struct ProbeConfig;

impl Config for ProbeConfig {}

struct ProbeResource;

impl Resource for ProbeResource {
    type Config = ProbeConfig;
    type Instance = u64;

    fn key(&self) -> ResourceKey {
        resource_key!("probe-pool-policy")
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

#[derive(Debug, Clone, Copy)]
struct ProbeResult {
    successes: u64,
    failures: u64,
    total_wait_ns: u128,
}

impl ProbeResult {
    fn merge(self, other: Self) -> Self {
        Self {
            successes: self.successes.saturating_add(other.successes),
            failures: self.failures.saturating_add(other.failures),
            total_wait_ns: self.total_wait_ns.saturating_add(other.total_wait_ns),
        }
    }
}

fn pool_config(max_size: usize, policy: PoolBackpressurePolicy) -> PoolConfig {
    PoolConfig {
        sizing: PoolSizing {
            min_size: 0,
            max_size,
        },
        lifetime: PoolLifetime {
            maintenance_interval: None,
            ..Default::default()
        },
        acquire: PoolAcquire {
            timeout: Duration::from_secs(2),
            backpressure: Some(policy),
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn run_probe(
    pool: Arc<Pool<ProbeResource>>,
    ctx: Context,
    workers: usize,
    runtime: Duration,
    hold_time: Duration,
) -> ProbeResult {
    let started = Instant::now();
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let pool = Arc::clone(&pool);
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            let mut out = ProbeResult {
                successes: 0,
                failures: 0,
                total_wait_ns: 0,
            };
            while started.elapsed() < runtime {
                match pool.acquire(&ctx).await {
                    Ok((guard, wait)) => {
                        out.successes = out.successes.saturating_add(1);
                        out.total_wait_ns = out.total_wait_ns.saturating_add(wait.as_nanos());
                        tokio::time::sleep(hold_time).await;
                        drop(guard);
                    }
                    Err(_) => {
                        out.failures = out.failures.saturating_add(1);
                    }
                }
                tokio::task::yield_now().await;
            }
            out
        }));
    }

    let mut total = ProbeResult {
        successes: 0,
        failures: 0,
        total_wait_ns: 0,
    };
    for h in handles {
        total = total.merge(h.await.expect("join"));
    }
    total
}

fn summary_metrics(result: ProbeResult) -> (u64, u64, u64, f64, f64) {
    let attempts = result.successes.saturating_add(result.failures);
    let success_rate = if attempts == 0 {
        0.0
    } else {
        (result.successes as f64) * 100.0 / (attempts as f64)
    };
    let avg_wait_us = if result.successes == 0 {
        0.0
    } else {
        (result.total_wait_ns as f64) / (result.successes as f64) / 1000.0
    };
    (
        attempts,
        result.successes,
        result.failures,
        success_rate,
        avg_wait_us,
    )
}

async fn run_policy_set(
    ctx: &Context,
    workers: usize,
    runtime: Duration,
    max_size: usize,
    hold_time: Duration,
) {
    let fail_fast = Arc::new(
        Pool::new(
            ProbeResource,
            ProbeConfig,
            pool_config(max_size, PoolBackpressurePolicy::FailFast),
        )
        .expect("pool"),
    );

    let bounded_wait = Arc::new(
        Pool::new(
            ProbeResource,
            ProbeConfig,
            pool_config(
                max_size,
                PoolBackpressurePolicy::BoundedWait {
                    timeout: Duration::from_millis(2),
                },
            ),
        )
        .expect("pool"),
    );

    let adaptive = Arc::new(
        Pool::new(
            ProbeResource,
            ProbeConfig,
            pool_config(
                max_size,
                PoolBackpressurePolicy::Adaptive(AdaptiveBackpressurePolicy {
                    high_pressure_utilization: 0.8,
                    high_pressure_waiters: 8,
                    low_pressure_timeout: Duration::from_millis(20),
                    high_pressure_timeout: Duration::from_millis(2),
                }),
            ),
        )
        .expect("pool"),
    );

    let ff = run_probe(
        Arc::clone(&fail_fast),
        ctx.clone(),
        workers,
        runtime,
        hold_time,
    )
    .await;
    let bw = run_probe(
        Arc::clone(&bounded_wait),
        ctx.clone(),
        workers,
        runtime,
        hold_time,
    )
    .await;
    let ad = run_probe(
        Arc::clone(&adaptive),
        ctx.clone(),
        workers,
        runtime,
        hold_time,
    )
    .await;

    let ffm = summary_metrics(ff);
    let bwm = summary_metrics(bw);
    let adm = summary_metrics(ad);

    println!(
        "{max_size},{:?},fail_fast,{},{},{},{:.2},{:.2}",
        hold_time, ffm.0, ffm.1, ffm.2, ffm.3, ffm.4
    );
    println!(
        "{max_size},{:?},bounded_wait,{},{},{},{:.2},{:.2}",
        hold_time, bwm.0, bwm.1, bwm.2, bwm.3, bwm.4
    );
    println!(
        "{max_size},{:?},adaptive,{},{},{},{:.2},{:.2}",
        hold_time, adm.0, adm.1, adm.2, adm.3, adm.4
    );
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let workers = 64;
    let runtime = Duration::from_secs(1);
    let max_sizes = [4usize, 8, 16];
    let hold_times = [
        Duration::from_micros(50),
        Duration::from_micros(200),
        Duration::from_micros(500),
    ];
    let ctx = Context::background(Scope::Global, WorkflowId::new(), ExecutionId::new());

    println!("pool_policy_probe matrix: workers={workers} runtime={runtime:?}");
    println!("max_size,hold_time,policy,attempts,success,fail,success_rate_pct,avg_wait_us");

    for max_size in max_sizes {
        for hold_time in hold_times {
            run_policy_set(&ctx, workers, runtime, max_size, hold_time).await;
        }
    }
}
