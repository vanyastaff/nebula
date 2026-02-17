//! Concurrent stress test for resource pool.
//!
//! Verifies that the pool handles 50+ concurrent tasks doing random
//! acquire/release cycles without deadlock, counter corruption, or panics.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use tokio::task::JoinSet;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StressConfig;

impl Config for StressConfig {}

struct StressResource {
    create_count: AtomicU64,
}

impl StressResource {
    fn new() -> Self {
        Self {
            create_count: AtomicU64::new(0),
        }
    }
}

impl Resource for StressResource {
    type Config = StressConfig;
    type Instance = u64;

    fn id(&self) -> &str {
        "stress"
    }

    async fn create(&self, _config: &StressConfig, _ctx: &Context) -> Result<u64> {
        let id = self.create_count.fetch_add(1, Ordering::SeqCst);
        // Simulate small creation latency
        tokio::time::sleep(Duration::from_micros(100)).await;
        Ok(id)
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "stress-wf", "stress-ex")
}

// ---------------------------------------------------------------------------
// Test 7a: stress_50_tasks_random_acquire_release
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_50_tasks_random_acquire_release() {
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 10,
        acquire_timeout: Duration::from_secs(10),
        ..Default::default()
    };
    let pool = Arc::new(Pool::new(StressResource::new(), StressConfig, pool_config).unwrap());

    let success_count = Arc::new(AtomicU64::new(0));
    let mut set = JoinSet::new();

    for _ in 0..50 {
        let pool = Arc::clone(&pool);
        let success_count = Arc::clone(&success_count);
        set.spawn(async move {
            let ctx = ctx();
            // Each task does 20 acquire/release cycles
            for _ in 0..20 {
                let guard = pool.acquire(&ctx).await.expect("task should acquire");
                // Simulate some work
                tokio::time::sleep(Duration::from_millis(1)).await;
                let _val: u64 = *guard;
                drop(guard);
                // Small delay for return-to-pool
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            success_count.fetch_add(1, Ordering::SeqCst);
        });
    }

    // Wait for all tasks (timeout 30s as safety net against deadlock)
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while let Some(result) = tokio::time::timeout_at(deadline, set.join_next())
        .await
        .expect("stress test should not deadlock (30s timeout)")
    {
        result.expect("task should not panic");
    }

    assert_eq!(
        success_count.load(Ordering::SeqCst),
        50,
        "all 50 tasks should complete successfully"
    );

    // Give final guard drops time to complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stats = pool.stats();
    assert_eq!(
        stats.active, 0,
        "no instances should be active after all tasks complete"
    );
    // 50 tasks * 20 cycles = 1000 acquisitions
    assert_eq!(stats.total_acquisitions, 1000);
    assert_eq!(
        stats.total_releases, stats.total_acquisitions,
        "total releases should match total acquisitions"
    );
    assert!(
        stats.destroyed <= stats.created,
        "destroyed ({}) should not exceed created ({})",
        stats.destroyed,
        stats.created
    );
}
