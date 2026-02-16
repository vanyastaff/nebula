//! Pool acquire() cancellation safety tests.
//!
//! Verifies that cancelling an acquire mid-wait does not leak semaphore
//! permits or corrupt pool state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

struct SimpleResource {
    counter: AtomicU64,
}

impl SimpleResource {
    fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }
}

impl Resource for SimpleResource {
    type Config = TestConfig;
    type Instance = u64;

    fn id(&self) -> &str {
        "simple"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<u64> {
        Ok(self.counter.fetch_add(1, Ordering::SeqCst))
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

// ---------------------------------------------------------------------------
// Test 6a: acquire_cancelled_mid_wait_no_slot_leak
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn acquire_cancelled_mid_wait_no_slot_leak() {
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 1,
        acquire_timeout: Duration::from_secs(30),
        ..Default::default()
    };
    let pool = Pool::new(SimpleResource::new(), TestConfig, pool_config).unwrap();

    // Hold the only slot
    let g1 = pool.acquire(&ctx()).await.unwrap();

    // Start a second acquire that will block waiting for the semaphore.
    // Use a CancellationToken to cancel it after 10ms.
    let token = CancellationToken::new();
    let cancel_ctx = Context::new(Scope::Global, "wf", "ex").with_cancellation(token.clone());

    let pool_clone = pool.clone();
    let handle = tokio::spawn(async move { pool_clone.acquire(&cancel_ctx).await });

    // Let the acquire start waiting on the semaphore
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Cancel it
    token.cancel();

    let result = handle.await.unwrap();
    assert!(result.is_err(), "cancelled acquire should fail");

    // Release the first guard
    drop(g1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Third acquire must succeed: the cancelled acquire must NOT have
    // consumed the semaphore permit
    let g3 = pool
        .acquire(&ctx())
        .await
        .expect("pool should still work after cancelled acquire");
    assert_eq!(*g3, 0, "should reuse the returned instance");

    drop(g3);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = pool.stats();
    assert_eq!(stats.active, 0);
}
