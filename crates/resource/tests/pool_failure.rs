//! Pool create() failure handling tests.
//!
//! Verifies that when `Resource::create()` returns `Err`, the pool remains
//! in a consistent state: semaphore permits are not leaked, counters are
//! correct, and subsequent acquires work normally.

use std::sync::atomic::{AtomicU32, Ordering};
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

fn pool_config(max_size: usize) -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Resource that always fails create
// ---------------------------------------------------------------------------

struct AlwaysFailResource;

impl Resource for AlwaysFailResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "always-fail"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Err(Error::Initialization {
            resource_id: "always-fail".to_string(),
            reason: "intentional failure".to_string(),
            source: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Resource that fails create on specific calls
// ---------------------------------------------------------------------------

struct IntermittentResource {
    /// Bitmask: if bit N is set, call N fails (0-indexed).
    fail_mask: u32,
    call_count: AtomicU32,
}

impl IntermittentResource {
    fn new(fail_mask: u32) -> Self {
        Self {
            fail_mask,
            call_count: AtomicU32::new(0),
        }
    }
}

impl Resource for IntermittentResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "intermittent"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        if self.fail_mask & (1 << n) != 0 {
            return Err(Error::Initialization {
                resource_id: "intermittent".to_string(),
                reason: format!("intentional failure on call {n}"),
                source: None,
            });
        }
        Ok(format!("inst-{n}"))
    }
}

// ---------------------------------------------------------------------------
// Test 1a: create_failure_does_not_corrupt_pool_state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_failure_does_not_corrupt_pool_state() {
    let pool = Pool::new(AlwaysFailResource, TestConfig, pool_config(2)).unwrap();

    // Acquire should fail (create returns Err)
    let result = pool.acquire(&ctx()).await;
    assert!(result.is_err());

    // Pool state should be clean
    let stats = pool.stats();
    assert_eq!(stats.active, 0, "no active instances after failed create");
    assert_eq!(stats.idle, 0, "no idle instances after failed create");

    // Semaphore permit should not be leaked: we can still do max_size acquires
    // with a resource that succeeds. But since AlwaysFailResource always fails,
    // we verify via stats that the permit was returned.
    // A leaked permit would eventually exhaust the pool even though active == 0.

    // Try again - should also fail but not deadlock or panic
    let result = pool.acquire(&ctx()).await;
    assert!(result.is_err());

    let stats = pool.stats();
    assert_eq!(stats.active, 0);
}

// ---------------------------------------------------------------------------
// Test 1b: create_failure_after_expired_cleanup
// ---------------------------------------------------------------------------

/// Resource that tracks create call count, failing on specified call.
struct ExpiredThenFailResource {
    call_count: AtomicU32,
}

impl Resource for ExpiredThenFailResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "expired-then-fail"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        if n == 1 {
            // Second create (after expired entry cleanup) fails
            return Err(Error::Initialization {
                resource_id: "expired-then-fail".to_string(),
                reason: "fail on replacement create".to_string(),
                source: None,
            });
        }
        Ok(format!("inst-{n}"))
    }
}

#[tokio::test]
async fn create_failure_after_expired_cleanup() {
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_millis(30),
        ..Default::default()
    };
    let pool = Pool::new(
        ExpiredThenFailResource {
            call_count: AtomicU32::new(0),
        },
        TestConfig,
        pool_config,
    )
    .unwrap();

    // First acquire succeeds (call 0)
    {
        let guard = pool.acquire(&ctx()).await.unwrap();
        assert_eq!(*guard, "inst-0");
    }
    // Return to pool
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Wait for idle timeout to expire the entry
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Next acquire: expired entry gets cleaned up, then create (call 1) fails
    let result = pool.acquire(&ctx()).await;
    assert!(result.is_err(), "create after expired cleanup should fail");

    let stats = pool.stats();
    assert_eq!(stats.active, 0, "no active instances after failure");
    assert!(
        stats.destroyed >= 1,
        "expired entry should have been destroyed"
    );

    // Pool should recover: next create (call 2) succeeds
    let guard = pool.acquire(&ctx()).await.expect("pool should recover");
    assert_eq!(*guard, "inst-2");
}

// ---------------------------------------------------------------------------
// Test 1c: intermittent_create_failure_recovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn intermittent_create_failure_recovery() {
    // Fail on calls 0, 1, 2 (first 3 calls), succeed from call 3 onwards
    let resource = IntermittentResource::new(0b0000_0111);
    let pool = Pool::new(resource, TestConfig, pool_config(2)).unwrap();

    // First 3 acquires should fail
    for i in 0..3 {
        let result = pool.acquire(&ctx()).await;
        assert!(result.is_err(), "acquire {i} should fail");
    }

    // Pool should not be corrupted
    let stats = pool.stats();
    assert_eq!(stats.active, 0);

    // Fourth acquire should succeed (call 3 succeeds)
    let guard = pool
        .acquire(&ctx())
        .await
        .expect("pool should recover after transient failures");
    assert_eq!(*guard, "inst-3");

    let stats = pool.stats();
    assert_eq!(stats.active, 1);
    assert_eq!(stats.created, 1, "only one successful create");

    // Can acquire second instance too (max_size=2)
    let guard2 = pool
        .acquire(&ctx())
        .await
        .expect("second acquire should work");
    assert_eq!(*guard2, "inst-4");

    let stats = pool.stats();
    assert_eq!(stats.active, 2);
    assert_eq!(stats.created, 2);
}
