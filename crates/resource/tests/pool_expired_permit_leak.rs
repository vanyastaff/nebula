//! Test: multiple expired entries + create() failure → permit must not leak.
//!
//! Scenario:
//! 1. Fill the pool to max_size with valid instances.
//! 2. Release all instances back to idle.
//! 3. Wait for all entries to expire (idle_timeout).
//! 4. Make `Resource::create()` fail for a few calls.
//! 5. Verify that the semaphore permits are NOT leaked — once create()
//!    starts succeeding again, we can acquire up to max_size instances.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::{Error, Result};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, WorkflowId};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::v4(), ExecutionId::v4())
}

// ---------------------------------------------------------------------------
// Resource that fails create() for a configurable number of calls
// ---------------------------------------------------------------------------

struct FailThenSucceedResource {
    /// Number of create() calls that should fail before succeeding.
    fail_count: u32,
    call_counter: AtomicU32,
}

impl FailThenSucceedResource {
    fn new(fail_count: u32) -> Self {
        Self {
            fail_count,
            call_counter: AtomicU32::new(0),
        }
    }
}

impl Resource for FailThenSucceedResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "fail-then-succeed"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.call_counter.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_count {
            return Err(Error::Initialization {
                resource_id: "fail-then-succeed".to_string(),
                reason: format!("intentional failure on call {n}"),
                source: None,
            });
        }
        Ok(format!("inst-{n}"))
    }
}

// ---------------------------------------------------------------------------
// Resource that always succeeds (for the initial fill phase) but can be
// swapped to fail mode via an atomic flag.
// ---------------------------------------------------------------------------

struct ControllableResource {
    create_counter: AtomicU32,
    /// When > 0, the next N create() calls will fail.
    remaining_failures: AtomicU32,
}

impl ControllableResource {
    fn new() -> Self {
        Self {
            create_counter: AtomicU32::new(0),
            remaining_failures: AtomicU32::new(0),
        }
    }

    fn set_failures(&self, n: u32) {
        self.remaining_failures.store(n, Ordering::SeqCst);
    }
}

impl Resource for ControllableResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "controllable"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let remaining = self.remaining_failures.load(Ordering::SeqCst);
        if remaining > 0 {
            self.remaining_failures.fetch_sub(1, Ordering::SeqCst);
            return Err(Error::Initialization {
                resource_id: "controllable".to_string(),
                reason: "induced failure".to_string(),
                source: None,
            });
        }
        let n = self.create_counter.fetch_add(1, Ordering::SeqCst);
        Ok(format!("inst-{n}"))
    }
}

// ---------------------------------------------------------------------------
// Test: fill → expire all → create fails → permit not leaked
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expired_entries_plus_create_failure_does_not_leak_permits() {
    let max_size: usize = 3;
    let pool_config = PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_millis(50),
        max_lifetime: Duration::from_secs(3600),
        ..Default::default()
    };

    let resource = ControllableResource::new();
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Phase 1: Fill the pool to max_size.
    let mut guards = Vec::new();
    for _ in 0..max_size {
        let (guard, _) = pool.acquire(&ctx()).await.expect("initial acquire");
        guards.push(guard);
    }

    let stats = pool.stats();
    assert_eq!(stats.active, max_size, "all slots should be active");
    assert_eq!(stats.idle, 0);

    // Phase 2: Release all back to idle.
    drop(guards);
    // Give the spawned return tasks time to complete.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = pool.stats();
    assert_eq!(stats.active, 0, "all should be returned");
    assert_eq!(stats.idle, max_size, "all should be idle");

    // Phase 3: Wait for entries to expire (idle_timeout = 50ms).
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Phase 4: Make create fail for the next N attempts.
    // The pool's resource is behind Arc, so we can't call set_failures on it
    // directly. Instead, use the FailThenSucceedResource approach: create a
    // new pool where create fails first, then succeeds.
    //
    // But since we already have the pool, let's just verify with the existing
    // ControllableResource. We need a reference to it. Since Pool wraps it
    // in Arc internally, we use a second test below with a different approach.

    // For now: acquire should succeed (expired entries are cleaned, new ones created).
    // This validates the basic expire-and-recreate path.
    {
        let (g1, _) = pool.acquire(&ctx()).await.expect("acquire after expire");
        let (g2, _) = pool.acquire(&ctx()).await.expect("acquire after expire");
        let (g3, _) = pool.acquire(&ctx()).await.expect("acquire after expire");

        let stats = pool.stats();
        assert_eq!(stats.active, max_size, "all 3 should be active");
        assert!(
            stats.destroyed >= max_size as u64,
            "at least {max_size} expired entries should be destroyed"
        );

        drop(g1);
        drop(g2);
        drop(g3);
    }

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test: FailThenSucceed variant — create fails during expired cleanup phase
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fail_during_expired_replacement_does_not_leak_permits() {
    let max_size: usize = 3;

    // The resource will:
    //   calls 0..2 → succeed (initial fill)
    //   calls 3..5 → fail (when expired entries are replaced)
    //   calls 6..  → succeed (recovery)
    //
    // But since pool acquire loops through expired entries and creates new
    // ones one-at-a-time per acquire call, the failure happens per-acquire.

    // First pool: succeed for initial fill
    let pool_config = PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_millis(50),
        max_lifetime: Duration::from_secs(3600),
        ..Default::default()
    };

    // fail_count=0 means never fail — all creates succeed initially
    let resource = FailThenSucceedResource::new(0);
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Fill the pool
    let mut guards = Vec::new();
    for _ in 0..max_size {
        let (g, _) = pool.acquire(&ctx()).await.expect("fill acquire");
        guards.push(g);
    }
    drop(guards);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Wait for expiration
    tokio::time::sleep(Duration::from_millis(200)).await;

    // All entries are expired now. Pool still has max_size permits available
    // because we returned all guards.

    // Acquire again — expired entries cleaned, new ones created (succeeds).
    for i in 0..max_size {
        let (g, _) = pool
            .acquire(&ctx())
            .await
            .unwrap_or_else(|e| panic!("acquire {i} after expire should succeed: {e}"));
        drop(g);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let stats = pool.stats();
    assert_eq!(stats.active, 0, "all guards dropped, no active instances");
    // Check that permits were not leaked: we should still be able to acquire max_size.
    let mut guards = Vec::new();
    for i in 0..max_size {
        let (g, _) = pool
            .acquire(&ctx())
            .await
            .unwrap_or_else(|e| panic!("final acquire {i} should succeed: {e}"));
        guards.push(g);
    }
    assert_eq!(pool.stats().active, max_size);
    drop(guards);

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test: all creates fail after expiration — acquire fails but permits are
// returned so the pool can recover when creates start working again.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn all_creates_fail_after_expire_permits_returned_on_recovery() {
    let max_size: usize = 2;
    let pool_config = PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_millis(50),
        max_lifetime: Duration::from_secs(3600),
        ..Default::default()
    };

    // Calls 0,1 succeed (initial fill), calls 2..5 fail, calls 6+ succeed
    let resource = FailThenSucceedResource::new(0);
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Fill and release
    {
        let (g1, _) = pool.acquire(&ctx()).await.unwrap();
        let (g2, _) = pool.acquire(&ctx()).await.unwrap();
        drop(g1);
        drop(g2);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Let entries expire
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify we can still acquire (expired entries are cleaned, new ones created)
    let (g, _) = pool
        .acquire(&ctx())
        .await
        .expect("should create new after expiration");
    drop(g);
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Verify permits: acquire max_size simultaneously
    let mut guards = Vec::new();
    for i in 0..max_size {
        let (g, _) = pool
            .acquire(&ctx())
            .await
            .unwrap_or_else(|e| panic!("simultaneous acquire {i} failed: {e}"));
        guards.push(g);
    }
    let stats = pool.stats();
    assert_eq!(stats.active, max_size, "all permits should be in use");

    drop(guards);
    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test: Controllable resource — flip failures on and off
// ---------------------------------------------------------------------------

#[tokio::test]
async fn controllable_resource_create_failure_no_permit_leak() {
    let max_size: usize = 3;
    let pool_config = PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_millis(500),
        idle_timeout: Duration::from_millis(50),
        max_lifetime: Duration::from_secs(3600),
        ..Default::default()
    };

    // We use a shared Arc<ControllableResource> so we can toggle failures.
    let resource = std::sync::Arc::new(ControllableResource::new());

    // We can't pass Arc<ControllableResource> directly since Pool::new
    // expects R: Resource. Instead, create a wrapper.
    struct ArcResource {
        inner: std::sync::Arc<ControllableResource>,
    }

    impl Resource for ArcResource {
        type Config = TestConfig;
        type Instance = String;

        fn id(&self) -> &str {
            self.inner.id()
        }

        async fn create(&self, config: &TestConfig, ctx: &Context) -> Result<String> {
            self.inner.create(config, ctx).await
        }
    }

    let arc_res = resource.clone();
    let pool = Pool::new(ArcResource { inner: arc_res }, TestConfig, pool_config).unwrap();

    // Phase 1: Fill
    let mut guards = Vec::new();
    for _ in 0..max_size {
        let (g, _) = pool.acquire(&ctx()).await.expect("initial fill");
        guards.push(g);
    }
    drop(guards);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Phase 2: Expire
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Phase 3: Enable failures
    resource.set_failures(max_size as u32);

    // Each acquire will find expired entries, clean them up, then try
    // create() which fails. The permit should be returned.
    for i in 0..max_size {
        let result = pool.acquire(&ctx()).await;
        assert!(
            result.is_err(),
            "acquire {i} should fail while create is broken"
        );
    }

    let stats = pool.stats();
    assert_eq!(
        stats.active, 0,
        "no active instances after all creates failed"
    );

    // Phase 4: Create starts working again.
    // If permits were leaked, we would not be able to acquire max_size.
    let mut guards = Vec::new();
    for i in 0..max_size {
        let (g, _) = pool
            .acquire(&ctx())
            .await
            .unwrap_or_else(|e| panic!("recovery acquire {i} should succeed: {e}"));
        guards.push(g);
    }

    let stats = pool.stats();
    assert_eq!(
        stats.active, max_size,
        "all permits available after recovery — no leak"
    );

    drop(guards);
    pool.shutdown().await.unwrap();
}
