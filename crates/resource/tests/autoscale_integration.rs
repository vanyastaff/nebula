//! Integration tests for the auto-scaling system.
//!
//! Covers tasks T076 (scale up), T077 (scale down), T078 (bounds + validation).
//!
//! These tests exercise the *real* `Pool` and `AutoScaler` types end-to-end,
//! wiring `Pool::scale_up`, `Pool::scale_down`, and `Pool::utilization_snapshot`
//! into the `AutoScaler`'s closure interface.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use nebula_resource::autoscale::{AutoScalePolicy, AutoScaler};
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// ---------------------------------------------------------------------------
// Test resource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig;

impl Config for TestConfig {}

struct CountingResource {
    create_count: Arc<AtomicU32>,
    cleanup_count: Arc<AtomicU32>,
}

impl CountingResource {
    fn new() -> (Self, Arc<AtomicU32>, Arc<AtomicU32>) {
        let create_count = Arc::new(AtomicU32::new(0));
        let cleanup_count = Arc::new(AtomicU32::new(0));
        (
            Self {
                create_count: Arc::clone(&create_count),
                cleanup_count: Arc::clone(&cleanup_count),
            },
            create_count,
            cleanup_count,
        )
    }
}

impl Resource for CountingResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "counting"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        let n = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(format!("inst-{n}"))
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
// T076: Pool scales up when utilization > high_watermark
// ===========================================================================

/// Direct test of Pool::scale_up creating idle instances.
#[tokio::test(flavor = "multi_thread")]
async fn pool_scale_up_creates_idle_instances() {
    let (resource, create_count, _cleanup) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 5,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Pool starts empty.
    assert_eq!(pool.stats().idle, 0);

    // Scale up by 3.
    let created = pool.scale_up(3).await;
    assert_eq!(created, 3, "should have created 3 instances");
    assert_eq!(create_count.load(Ordering::SeqCst), 3);

    let stats = pool.stats();
    assert_eq!(stats.idle, 3, "all 3 should be idle");
    assert_eq!(stats.active, 0);
}

/// AutoScaler triggers scale_up on a real pool when utilization is high.
#[tokio::test(flavor = "multi_thread")]
async fn pool_scales_up_on_high_utilization() {
    let (resource, create_count, _cleanup) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 1,
        max_size: 8,
        acquire_timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Acquire 4 out of 8 -> 50% utilization.
    // We want high_watermark at 0.4, so 50% > 40% triggers scale-up.
    let mut guards = Vec::new();
    for _ in 0..4 {
        guards.push(pool.acquire(&ctx()).await.unwrap());
    }
    assert_eq!(pool.stats().active, 4);

    // Wire the auto-scaler to the real pool.
    let cancel = CancellationToken::new();
    let policy = AutoScalePolicy {
        high_watermark: 0.4,
        low_watermark: 0.1,
        scale_up_step: 2,
        scale_down_step: 1,
        // Use very short windows so the test finishes quickly.
        evaluation_window: Duration::from_millis(80),
        cooldown: Duration::from_millis(50),
    };
    let scaler = AutoScaler::new(policy, cancel.clone());

    let stats_pool = pool.clone();
    let up_pool = pool.clone();
    let down_pool = pool.clone();

    scaler.start(
        move || stats_pool.utilization_snapshot(),
        move |n| {
            let p = up_pool.clone();
            async move { p.scale_up(n).await }
        },
        move |n| {
            let p = down_pool.clone();
            async move { p.scale_down(n).await }
        },
    );

    // Wait for sustained high + scale action.
    // evaluation_window=80ms, check_interval=40ms.
    // First check at 40ms detects high; second at 80ms may not yet meet window;
    // by ~120-160ms it should trigger.
    tokio::time::sleep(Duration::from_millis(300)).await;

    scaler.shutdown();

    let stats = pool.stats();
    // We started with 4 active + 0 idle. scale_up(2) should have added 2 idle.
    assert!(
        stats.idle >= 1,
        "auto-scaler should have pre-created idle instances, got idle={}",
        stats.idle
    );
    // Total creates: 4 (acquired) + at least 1 from scale_up.
    assert!(
        create_count.load(Ordering::SeqCst) > 4,
        "expected more than 4 creates, got {}",
        create_count.load(Ordering::SeqCst)
    );

    // Cleanup.
    drop(guards);
    tokio::time::sleep(Duration::from_millis(50)).await;
    pool.shutdown().await.unwrap();
}

// ===========================================================================
// T077: Pool scales down when utilization < low_watermark
// ===========================================================================

/// Direct test of Pool::scale_down removing idle instances.
#[tokio::test(flavor = "multi_thread")]
async fn pool_scale_down_removes_idle_instances() {
    let (resource, _create, cleanup_count) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 1,
        max_size: 10,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Pre-create 5 idle instances.
    let scaled = pool.scale_up(5).await;
    assert_eq!(scaled, 5);
    assert_eq!(pool.stats().idle, 5);

    // Scale down by 3. min_size=1, so at most 4 can be removed (5-1=4).
    let removed = pool.scale_down(3).await;
    assert_eq!(removed, 3, "should have removed 3 instances");
    assert_eq!(pool.stats().idle, 2);
    assert_eq!(cleanup_count.load(Ordering::SeqCst), 3);
}

/// AutoScaler triggers scale_down on a real pool when utilization is low.
#[tokio::test(flavor = "multi_thread")]
async fn pool_scales_down_on_low_utilization() {
    let (resource, _create, cleanup_count) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 1,
        max_size: 10,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Pre-create 5 idle instances, 0 active.
    // Utilization = 0/10 = 0% which is below low_watermark of 0.3.
    pool.scale_up(5).await;
    assert_eq!(pool.stats().idle, 5);

    let cancel = CancellationToken::new();
    let policy = AutoScalePolicy {
        high_watermark: 0.8,
        low_watermark: 0.3,
        scale_up_step: 1,
        scale_down_step: 2,
        evaluation_window: Duration::from_millis(80),
        cooldown: Duration::from_millis(50),
    };
    let scaler = AutoScaler::new(policy, cancel.clone());

    let stats_pool = pool.clone();
    let up_pool = pool.clone();
    let down_pool = pool.clone();

    scaler.start(
        move || stats_pool.utilization_snapshot(),
        move |n| {
            let p = up_pool.clone();
            async move { p.scale_up(n).await }
        },
        move |n| {
            let p = down_pool.clone();
            async move { p.scale_down(n).await }
        },
    );

    // Wait for scale-down to trigger.
    tokio::time::sleep(Duration::from_millis(300)).await;

    scaler.shutdown();

    let stats = pool.stats();
    assert!(
        stats.idle < 5,
        "auto-scaler should have removed some idle instances, got idle={}",
        stats.idle
    );
    assert!(
        cleanup_count.load(Ordering::SeqCst) >= 1,
        "expected at least 1 cleanup from scale_down, got {}",
        cleanup_count.load(Ordering::SeqCst)
    );

    pool.shutdown().await.unwrap();
}

// ===========================================================================
// T078: Auto-scaler respects min_size / max_size bounds
// ===========================================================================

/// scale_up cannot exceed max_size.
#[tokio::test(flavor = "multi_thread")]
async fn scale_up_capped_at_max_size() {
    let (resource, create_count, _cleanup) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 3,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Acquire 1, so 1 active + 0 idle = 1 total. max_size = 3.
    let _g = pool.acquire(&ctx()).await.unwrap();

    // scale_up(10) should only create 2 (3 - 1 = 2 headroom).
    let created = pool.scale_up(10).await;
    assert_eq!(created, 2, "should cap at max_size");
    assert_eq!(create_count.load(Ordering::SeqCst), 3); // 1 from acquire + 2 from scale_up

    let stats = pool.stats();
    assert_eq!(stats.idle, 2);
    assert_eq!(stats.active, 1);
}

/// scale_down cannot go below min_size.
#[tokio::test(flavor = "multi_thread")]
async fn scale_down_capped_at_min_size() {
    let (resource, _create, cleanup_count) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 2,
        max_size: 10,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Pre-create 5 idle instances. 0 active + 5 idle = 5 total.
    pool.scale_up(5).await;
    assert_eq!(pool.stats().idle, 5);

    // scale_down(10) should only remove 3 (5 - min_size(2) = 3).
    let removed = pool.scale_down(10).await;
    assert_eq!(removed, 3, "should not go below min_size");
    assert_eq!(cleanup_count.load(Ordering::SeqCst), 3);

    let stats = pool.stats();
    assert_eq!(stats.idle, 2, "should have exactly min_size remaining");
}

/// scale_down with active instances counts towards min_size.
#[tokio::test(flavor = "multi_thread")]
async fn scale_down_counts_active_towards_min() {
    let (resource, _create, cleanup_count) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 3,
        max_size: 10,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Acquire 1 active, then scale_up 4 idle. Total = 5.
    let _g = pool.acquire(&ctx()).await.unwrap();
    pool.scale_up(4).await;
    assert_eq!(pool.stats().active, 1);
    assert_eq!(pool.stats().idle, 4);

    // scale_down(10): total = 5, min_size = 3, so can remove 2.
    let removed = pool.scale_down(10).await;
    assert_eq!(removed, 2, "active counts towards min_size");
    assert_eq!(cleanup_count.load(Ordering::SeqCst), 2);

    let stats = pool.stats();
    assert_eq!(stats.idle, 2);
    assert_eq!(stats.active, 1);
    // total = 3 = min_size
}

/// scale_up on an already-full pool creates nothing.
#[tokio::test(flavor = "multi_thread")]
async fn scale_up_on_full_pool_is_noop() {
    let (resource, _create, _cleanup) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Fill up to max_size with idle instances.
    pool.scale_up(2).await;
    assert_eq!(pool.stats().idle, 2);

    // Another scale_up should do nothing.
    let created = pool.scale_up(5).await;
    assert_eq!(created, 0, "pool is already at max_size");
    assert_eq!(pool.stats().idle, 2);
}

/// scale_down on an empty pool is a no-op.
#[tokio::test(flavor = "multi_thread")]
async fn scale_down_on_empty_pool_is_noop() {
    let (resource, _create, _cleanup) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 5,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    let removed = pool.scale_down(5).await;
    assert_eq!(removed, 0);
}

// ===========================================================================
// T078 continued: AutoScalePolicy validation
// ===========================================================================

#[test]
fn autoscale_policy_high_watermark_above_one_rejected() {
    let policy = AutoScalePolicy {
        high_watermark: 1.5,
        ..Default::default()
    };
    assert!(
        policy.validate().is_err(),
        "high_watermark > 1.0 should be invalid"
    );
}

#[test]
fn autoscale_policy_low_ge_high_rejected() {
    let policy = AutoScalePolicy {
        low_watermark: 0.9,
        high_watermark: 0.5,
        ..Default::default()
    };
    assert!(
        policy.validate().is_err(),
        "low_watermark >= high_watermark should be invalid"
    );
}

#[test]
fn autoscale_policy_low_equals_high_rejected() {
    let policy = AutoScalePolicy {
        low_watermark: 0.5,
        high_watermark: 0.5,
        ..Default::default()
    };
    assert!(
        policy.validate().is_err(),
        "low_watermark == high_watermark should be invalid"
    );
}

#[test]
fn autoscale_policy_scale_up_step_zero_rejected() {
    let policy = AutoScalePolicy {
        scale_up_step: 0,
        ..Default::default()
    };
    assert!(
        policy.validate().is_err(),
        "scale_up_step == 0 should be invalid"
    );
}

#[test]
fn autoscale_policy_scale_down_step_zero_rejected() {
    let policy = AutoScalePolicy {
        scale_down_step: 0,
        ..Default::default()
    };
    assert!(
        policy.validate().is_err(),
        "scale_down_step == 0 should be invalid"
    );
}

#[test]
fn autoscale_policy_evaluation_window_zero_rejected() {
    let policy = AutoScalePolicy {
        evaluation_window: Duration::ZERO,
        ..Default::default()
    };
    assert!(
        policy.validate().is_err(),
        "evaluation_window == 0 should be invalid"
    );
}

#[test]
fn autoscale_policy_valid_params_accepted() {
    let policy = AutoScalePolicy {
        high_watermark: 0.8,
        low_watermark: 0.2,
        scale_up_step: 3,
        scale_down_step: 1,
        evaluation_window: Duration::from_secs(30),
        cooldown: Duration::from_secs(60),
    };
    assert!(policy.validate().is_ok(), "valid policy should pass");
}

#[test]
fn autoscale_policy_default_is_valid() {
    AutoScalePolicy::default()
        .validate()
        .expect("default policy should be valid");
}

/// Boundary: high_watermark exactly 1.0 is valid (means "never scale up" in practice).
#[test]
fn autoscale_policy_high_watermark_one_is_valid() {
    let policy = AutoScalePolicy {
        high_watermark: 1.0,
        low_watermark: 0.5,
        ..Default::default()
    };
    assert!(policy.validate().is_ok());
}

/// Boundary: low_watermark exactly 0.0 is valid (means "never scale down" in practice).
#[test]
fn autoscale_policy_low_watermark_zero_is_valid() {
    let policy = AutoScalePolicy {
        low_watermark: 0.0,
        high_watermark: 0.5,
        ..Default::default()
    };
    assert!(policy.validate().is_ok());
}

// ===========================================================================
// Wiring test: Pool::utilization_snapshot returns correct values
// ===========================================================================

#[tokio::test(flavor = "multi_thread")]
async fn utilization_snapshot_reflects_pool_state() {
    let (resource, _create, _cleanup) = CountingResource::new();
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 10,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(resource, TestConfig, pool_config).unwrap();

    // Empty pool.
    let (active, idle, max) = pool.utilization_snapshot();
    assert_eq!((active, idle, max), (0, 0, 10));

    // Acquire 2.
    let _g1 = pool.acquire(&ctx()).await.unwrap();
    let _g2 = pool.acquire(&ctx()).await.unwrap();
    let (active, idle, max) = pool.utilization_snapshot();
    assert_eq!((active, idle, max), (2, 0, 10));

    // Pre-create 3 idle.
    pool.scale_up(3).await;
    let (active, idle, max) = pool.utilization_snapshot();
    assert_eq!((active, idle, max), (2, 3, 10));
}
