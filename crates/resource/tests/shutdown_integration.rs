//! T017: Phased shutdown drains idle, post-shutdown guard drops clean up.
//!
//! Verifies:
//! 1. Shutdown closes the pool, cleans up idle instances
//! 2. Guards dropped after shutdown are cleaned up (not returned to idle)
//! 3. New acquires after shutdown fail immediately

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// ---------------------------------------------------------------------------
// Test resource that tracks cleanup count
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig;

impl Config for TestConfig {}

struct TrackingResource {
    cleanup_count: Arc<AtomicU32>,
}

impl Resource for TrackingResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "tracked"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok("tracked-instance".to_string())
    }

    async fn cleanup(&self, _instance: String) -> Result<()> {
        self.cleanup_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

/// Shutdown cleans up idle instances, then guard dropped post-shutdown
/// triggers cleanup (not return-to-pool).
#[tokio::test(start_paused = true)]
async fn shutdown_cleans_idle_then_guard_drop_cleans_active() {
    let cleanup_count = Arc::new(AtomicU32::new(0));

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(
        TrackingResource {
            cleanup_count: cleanup_count.clone(),
        },
        TestConfig,
        pool_config,
    )
    .unwrap();

    // Acquire two instances
    let g1 = pool.acquire(&ctx()).await.unwrap();
    let g2 = pool.acquire(&ctx()).await.unwrap();

    // Return g1 to create an idle instance
    drop(g1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = pool.stats();
    assert_eq!(stats.idle, 1, "one instance should be idle");
    assert_eq!(stats.active, 1, "one instance should be active");

    // Shutdown: cleans up idle instance, marks pool as shut down
    pool.shutdown().await.unwrap();

    let stats = pool.stats();
    assert_eq!(stats.idle, 0, "idle should be 0 after shutdown");
    assert!(
        cleanup_count.load(Ordering::SeqCst) >= 1,
        "at least one cleanup should have happened (idle instance)"
    );

    let cleanups_after_shutdown = cleanup_count.load(Ordering::SeqCst);

    // Drop the remaining guard: should be cleaned up (not returned to idle)
    drop(g2);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let stats = pool.stats();
    assert_eq!(
        stats.idle, 0,
        "instance dropped after shutdown should NOT go to idle"
    );
    assert!(
        cleanup_count.load(Ordering::SeqCst) > cleanups_after_shutdown,
        "guard dropped after shutdown should trigger cleanup"
    );
}

/// Manager-level phased shutdown cleans up all pools and clears entries.
#[tokio::test(start_paused = true)]
async fn manager_shutdown_clears_all_pools() {
    let cleanup_count = Arc::new(AtomicU32::new(0));

    let mgr = Manager::new();
    mgr.register(
        TrackingResource {
            cleanup_count: cleanup_count.clone(),
        },
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap();

    // Acquire and release to create an idle instance
    {
        let _g = mgr.acquire("tracked", &ctx()).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Shutdown should clean up idle and clear pools
    mgr.shutdown().await.unwrap();

    assert!(
        cleanup_count.load(Ordering::SeqCst) >= 1,
        "idle instance should be cleaned up during shutdown"
    );

    // After shutdown, acquire should fail (pool entry removed)
    let result = mgr.acquire("tracked", &ctx()).await;
    assert!(
        result.is_err(),
        "acquire after manager shutdown should fail"
    );
}

/// Shutdown with no instances is a no-op.
#[tokio::test]
async fn shutdown_empty_pool_is_noop() {
    let cleanup_count = Arc::new(AtomicU32::new(0));

    let pool = Pool::new(
        TrackingResource {
            cleanup_count: cleanup_count.clone(),
        },
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .unwrap();

    // No instances acquired
    pool.shutdown().await.unwrap();

    assert_eq!(
        cleanup_count.load(Ordering::SeqCst),
        0,
        "no cleanup needed for empty pool"
    );

    let stats = pool.stats();
    assert_eq!(stats.idle, 0);
    assert_eq!(stats.active, 0);
}

/// New acquire calls after shutdown fail immediately (not after timeout).
#[tokio::test]
async fn acquire_after_pool_shutdown_fails_immediately() {
    let pool = Pool::new(
        TrackingResource {
            cleanup_count: Arc::new(AtomicU32::new(0)),
        },
        TestConfig,
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(10), // long timeout
            ..Default::default()
        },
    )
    .unwrap();

    pool.shutdown().await.unwrap();

    let start = std::time::Instant::now();
    let result = pool.acquire(&ctx()).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "acquire after shutdown should fail");
    assert!(
        elapsed < Duration::from_secs(1),
        "should fail immediately, not wait for timeout (took {:?})",
        elapsed
    );
}
