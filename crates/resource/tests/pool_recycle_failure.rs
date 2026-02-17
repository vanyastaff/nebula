//! Pool recycle() error path tests.
//!
//! Verifies that when `Resource::recycle()` returns `Err`, the instance is
//! cleaned up instead of returned to the idle pool, and the pool remains
//! functional for subsequent acquires.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::{Error, Result};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

// ---------------------------------------------------------------------------
// Resource with controllable recycle failure
// ---------------------------------------------------------------------------

struct RecycleResource {
    fail_recycle: Arc<AtomicBool>,
    create_count: Arc<AtomicU32>,
}

impl RecycleResource {
    fn new(fail_recycle: Arc<AtomicBool>) -> Self {
        Self {
            fail_recycle,
            create_count: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl Resource for RecycleResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "recycle-test"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(format!("inst-{n}"))
    }

    async fn recycle(&self, _instance: &mut String) -> Result<()> {
        if self.fail_recycle.load(Ordering::SeqCst) {
            return Err(Error::Internal {
                resource_id: "recycle-test".to_string(),
                message: "recycle failed".to_string(),
                source: None,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test 3a: recycle_failure_destroys_instance
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn recycle_failure_destroys_instance() {
    let fail_flag = Arc::new(AtomicBool::new(true)); // always fail recycle
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(RecycleResource::new(fail_flag), TestConfig, pool_config).unwrap();

    // Acquire and drop: recycle fails → instance destroyed, not returned to idle
    {
        let guard = pool.acquire(&ctx()).await.unwrap();
        assert_eq!(*guard, "inst-0");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = pool.stats();
    assert_eq!(
        stats.destroyed, 1,
        "failed recycle should destroy the instance"
    );
    assert_eq!(
        stats.idle, 0,
        "destroyed instance should not be in idle pool"
    );
    assert_eq!(
        stats.active, 0,
        "instance should no longer be active after release"
    );
}

// ---------------------------------------------------------------------------
// Test 3b: recycle_failure_does_not_block_next_acquire
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn recycle_failure_does_not_block_next_acquire() {
    let fail_flag = Arc::new(AtomicBool::new(true)); // recycle fails
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 1, // only 1 slot!
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let resource = RecycleResource::new(fail_flag.clone());
    let create_count = resource.create_count.clone();
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Acquire and drop: recycle fails → instance destroyed, permit returned
    {
        let guard = pool.acquire(&ctx()).await.unwrap();
        assert_eq!(*guard, "inst-0");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(pool.stats().destroyed, 1);

    // Next acquire should work: creates a fresh instance (permit was returned)
    let guard = pool
        .acquire(&ctx())
        .await
        .expect("pool should be usable after recycle failure");
    assert_eq!(*guard, "inst-1");
    assert_eq!(create_count.load(Ordering::SeqCst), 2);

    // Now disable recycle failure and verify normal path works
    fail_flag.store(false, Ordering::SeqCst);
    drop(guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = pool.stats();
    assert_eq!(
        stats.idle, 1,
        "instance should be in idle pool after successful recycle"
    );
    assert_eq!(stats.destroyed, 1, "no additional destroys");
}
