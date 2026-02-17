//! Pool is_valid() rejection path tests.
//!
//! Verifies that when `Resource::is_valid()` returns `false` or `Err` for
//! idle instances, the pool correctly discards them and creates replacements.

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
// Shared resource with controllable validation
// ---------------------------------------------------------------------------

struct SharedValidatableResource {
    reject: Arc<AtomicBool>,
    create_count: Arc<AtomicU32>,
}

impl Resource for SharedValidatableResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "validatable"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(format!("inst-{n}"))
    }

    async fn is_valid(&self, _instance: &String) -> Result<bool> {
        if self.reject.swap(false, Ordering::SeqCst) {
            return Ok(false);
        }
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Shared resource that rejects all validation when flag is set
// ---------------------------------------------------------------------------

struct MultiInvalidResource {
    create_count: Arc<AtomicU32>,
    reject_all: Arc<AtomicBool>,
}

impl Resource for MultiInvalidResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "multi-invalid"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(format!("inst-{n}"))
    }

    async fn is_valid(&self, _instance: &String) -> Result<bool> {
        if self.reject_all.load(Ordering::SeqCst) {
            return Ok(false);
        }
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Shared resource where is_valid returns Err when flag is set
// ---------------------------------------------------------------------------

struct ErrValidResource {
    error_flag: Arc<AtomicBool>,
    create_count: Arc<AtomicU32>,
}

impl Resource for ErrValidResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "err-valid"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(format!("inst-{n}"))
    }

    async fn is_valid(&self, _instance: &String) -> Result<bool> {
        if self.error_flag.swap(false, Ordering::SeqCst) {
            return Err(Error::HealthCheck {
                resource_id: "err-valid".to_string(),
                reason: "validation error".to_string(),
                attempt: 1,
            });
        }
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Test 2a: invalid_idle_instance_replaced_on_acquire
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn invalid_idle_instance_replaced_on_acquire() {
    let reject = Arc::new(AtomicBool::new(false));
    let create_count = Arc::new(AtomicU32::new(0));

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(
        SharedValidatableResource {
            reject: reject.clone(),
            create_count: create_count.clone(),
        },
        TestConfig,
        pool_config,
    )
    .unwrap();

    // Acquire and release to put instance in idle pool
    {
        let guard = pool.acquire(&ctx()).await.unwrap();
        assert_eq!(*guard, "inst-0");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(pool.stats().idle, 1, "instance should be idle");

    // Set reject flag so next is_valid returns false (then auto-resets)
    reject.store(true, Ordering::SeqCst);

    // Acquire: idle instance fails is_valid, gets destroyed, new one created
    let guard = pool.acquire(&ctx()).await.unwrap();
    assert_eq!(*guard, "inst-1", "should get a fresh instance");

    let stats = pool.stats();
    assert!(stats.destroyed >= 1, "invalid instance should be destroyed");
    assert_eq!(stats.created, 2, "original + replacement");
}

// ---------------------------------------------------------------------------
// Test 2b: all_idle_instances_invalid_creates_fresh
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn all_idle_instances_invalid_creates_fresh() {
    let create_count = Arc::new(AtomicU32::new(0));
    let reject_all = Arc::new(AtomicBool::new(false));

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 3,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(
        MultiInvalidResource {
            create_count: create_count.clone(),
            reject_all: reject_all.clone(),
        },
        TestConfig,
        pool_config,
    )
    .unwrap();

    // Acquire and release 3 instances to fill idle pool
    for _ in 0..3 {
        let _g = pool.acquire(&ctx()).await.unwrap();
        // drop returns to pool
    }
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stats = pool.stats();
    assert!(stats.idle >= 1, "should have idle instances");
    let created_before = stats.created;

    // Now reject all validation
    reject_all.store(true, Ordering::SeqCst);

    // Acquire: all idle instances should be discarded, one new created
    let guard = pool.acquire(&ctx()).await.unwrap();

    let stats = pool.stats();
    assert!(
        stats.destroyed >= 1,
        "at least some invalid idle instances should be destroyed"
    );
    assert!(
        stats.created > created_before,
        "should have created a replacement instance"
    );

    // The acquired instance is fresh
    assert!((*guard).starts_with("inst-"), "should be a valid instance");
}

// ---------------------------------------------------------------------------
// Test 2c: is_valid_error_treated_as_invalid
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn is_valid_error_treated_as_invalid() {
    let error_flag = Arc::new(AtomicBool::new(false));
    let create_count = Arc::new(AtomicU32::new(0));

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(
        ErrValidResource {
            error_flag: error_flag.clone(),
            create_count: create_count.clone(),
        },
        TestConfig,
        pool_config,
    )
    .unwrap();

    // Acquire and release to idle
    {
        let guard = pool.acquire(&ctx()).await.unwrap();
        assert_eq!(*guard, "inst-0");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Set error flag so is_valid returns Err
    error_flag.store(true, Ordering::SeqCst);

    // Acquire: is_valid returns Err -> treated same as false -> cleanup + new create
    let guard = pool.acquire(&ctx()).await.unwrap();
    assert_eq!(
        *guard, "inst-1",
        "should get a fresh instance after Err validation"
    );

    let stats = pool.stats();
    assert!(
        stats.destroyed >= 1,
        "error-validated instance should be destroyed"
    );
    assert_eq!(stats.created, 2, "original + replacement");
}
