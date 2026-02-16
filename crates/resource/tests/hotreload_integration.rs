//! Integration tests for configuration hot-reload (T079).
//!
//! Verifies that `Manager::reload_config` creates a new pool with the new
//! configuration, shuts down the old pool, and allows acquiring instances
//! under the new limits.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// ---------------------------------------------------------------------------
// Test resource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig;

impl Config for TestConfig {}

struct TrackingResource {
    cleanup_count: Arc<AtomicU32>,
}

impl TrackingResource {
    fn new() -> (Self, Arc<AtomicU32>) {
        let count = Arc::new(AtomicU32::new(0));
        (
            Self {
                cleanup_count: Arc::clone(&count),
            },
            count,
        )
    }

    fn with_counter(counter: &Arc<AtomicU32>) -> Self {
        Self {
            cleanup_count: Arc::clone(counter),
        }
    }
}

impl Resource for TrackingResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "reload-test"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok("instance".to_string())
    }

    async fn cleanup(&self, _instance: String) -> Result<()> {
        self.cleanup_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

// ===========================================================================
// T079: Config hot-reload creates new pool
// ===========================================================================

/// After reload_config, the new pool's max_size is in effect.
#[tokio::test(flavor = "multi_thread")]
async fn config_hot_reload_creates_new_pool() {
    let (resource, cleanup_count) = TrackingResource::new();

    let mgr = Manager::new();
    mgr.register(
        resource,
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap();

    // Prove the old pool works: acquire and release.
    {
        let guard = mgr.acquire("reload-test", &ctx()).await.unwrap();
        let inst = guard
            .as_any()
            .downcast_ref::<String>()
            .expect("should downcast");
        assert_eq!(inst, "instance");
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Reload with a larger max_size.
    mgr.reload_config(
        TrackingResource::with_counter(&cleanup_count),
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // The old pool was shut down, so its idle instance was cleaned up.
    assert!(
        cleanup_count.load(Ordering::SeqCst) >= 1,
        "old pool's idle instances should have been cleaned up"
    );

    // Now we should be able to acquire more than 2 resources (old max_size).
    let mut guards = Vec::new();
    for i in 0..4 {
        let g = mgr
            .acquire("reload-test", &ctx())
            .await
            .unwrap_or_else(|e| panic!("acquire #{i} should succeed with new max_size=5: {e}"));
        guards.push(g);
    }
    assert_eq!(guards.len(), 4, "should hold 4 guards under new max_size=5");
}

/// Reload with reduced max_size restricts further acquisitions.
#[tokio::test(flavor = "multi_thread")]
async fn config_hot_reload_reduces_max_size() {
    let (resource, cleanup_count) = TrackingResource::new();

    let mgr = Manager::new();
    mgr.register(
        resource,
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap();

    // Reload with smaller max_size.
    mgr.reload_config(
        TrackingResource::with_counter(&cleanup_count),
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_millis(200),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // Acquire 2 (should succeed).
    let _g1 = mgr.acquire("reload-test", &ctx()).await.unwrap();
    let _g2 = mgr.acquire("reload-test", &ctx()).await.unwrap();

    // Third acquire should fail (new max_size = 2).
    let result = mgr.acquire("reload-test", &ctx()).await;
    assert!(
        result.is_err(),
        "third acquire should fail with new max_size=2"
    );
}

/// Reload on an unregistered resource is treated as a fresh registration.
#[tokio::test(flavor = "multi_thread")]
async fn reload_config_on_unregistered_resource_registers_fresh() {
    let (resource, _cleanup) = TrackingResource::new();

    let mgr = Manager::new();

    // Resource is not registered yet. reload_config should accept it.
    mgr.reload_config(
        resource,
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 3,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // Should be acquirable now.
    let guard = mgr.acquire("reload-test", &ctx()).await.unwrap();
    let inst = guard
        .as_any()
        .downcast_ref::<String>()
        .expect("should downcast");
    assert_eq!(inst, "instance");
}

/// Reload with invalid config fails without disturbing the existing pool.
#[tokio::test(flavor = "multi_thread")]
async fn reload_config_with_invalid_config_fails_cleanly() {
    let (resource, cleanup_count) = TrackingResource::new();

    let mgr = Manager::new();
    mgr.register(
        resource,
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap();

    // Attempt reload with invalid config (max_size = 0).
    let result = mgr
        .reload_config(
            TrackingResource::with_counter(&cleanup_count),
            TestConfig,
            PoolConfig {
                min_size: 0,
                max_size: 0, // invalid
                acquire_timeout: Duration::from_secs(1),
                ..Default::default()
            },
        )
        .await;

    assert!(result.is_err(), "reload with max_size=0 should fail");

    // Original pool should still work.
    let guard = mgr.acquire("reload-test", &ctx()).await.unwrap();
    let inst = guard
        .as_any()
        .downcast_ref::<String>()
        .expect("should downcast");
    assert_eq!(inst, "instance", "original pool should still be intact");
}

/// Guards from the old pool clean up on drop instead of returning to idle.
#[tokio::test(flavor = "multi_thread")]
async fn old_pool_guards_cleanup_after_reload() {
    let (resource, cleanup_count) = TrackingResource::new();

    let mgr = Manager::new();
    mgr.register(
        resource,
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap();

    // Hold a guard from the old pool.
    let old_guard = mgr.acquire("reload-test", &ctx()).await.unwrap();

    // Reload config.
    mgr.reload_config(
        TrackingResource::with_counter(&cleanup_count),
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 3,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let cleanups_before = cleanup_count.load(Ordering::SeqCst);

    // Drop the old guard. Since the old pool is shut down, this should
    // trigger cleanup (not a return to pool).
    drop(old_guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        cleanup_count.load(Ordering::SeqCst) > cleanups_before,
        "dropping old guard after reload should trigger cleanup"
    );

    // New pool should still be functional.
    let _g = mgr.acquire("reload-test", &ctx()).await.unwrap();
}

/// Concurrent acquires during reload: some may fail transiently but pool
/// recovers. Documents the availability gap between remove and insert.
#[tokio::test(flavor = "multi_thread")]
async fn reload_config_concurrent_acquire() {
    let (resource, cleanup_count) = TrackingResource::new();

    let mgr = Arc::new(Manager::new());
    mgr.register(
        resource,
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 4,
            acquire_timeout: Duration::from_millis(200),
            ..Default::default()
        },
    )
    .unwrap();

    let mgr_acquirer = Arc::clone(&mgr);
    let acquire_handle = tokio::spawn(async move {
        let mut successes = 0u32;
        let mut failures = 0u32;
        for _ in 0..20 {
            match mgr_acquirer.acquire("reload-test", &ctx()).await {
                Ok(guard) => {
                    successes += 1;
                    drop(guard);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(_) => {
                    failures += 1;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }
        (successes, failures)
    });

    // Let acquirer start
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Reload in the middle of acquire loop
    mgr.reload_config(
        TrackingResource::with_counter(&cleanup_count),
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 4,
            acquire_timeout: Duration::from_millis(200),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let (successes, _failures) = acquire_handle.await.unwrap();

    // Most acquires should succeed; some may fail during the brief reload window
    assert!(
        successes >= 10,
        "at least half of acquires should succeed, got {successes}/20"
    );

    // After reload, pool should be fully functional
    let guard = mgr.acquire("reload-test", &ctx()).await.unwrap();
    let inst = guard
        .as_any()
        .downcast_ref::<String>()
        .expect("should downcast");
    assert_eq!(inst, "instance");
}
