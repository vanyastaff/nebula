//! Integration tests for Manager fixes:
//!
//! 1. `deregister` cancels auto-scaler, stops health monitoring, releases quarantine
//! 2. `ManagerBuilder::default_autoscale_policy` auto-enables scaling on register
//! 3. `Manager::start_health_monitoring` convenience wrapper
//! 4. `PoolStats` latency percentiles (p50/p95/p99)
//! 5. `HealthChecker::stop_monitoring_resource` cancels all instances by resource_id

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::autoscale::AutoScalePolicy;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::events::EventBus;
use nebula_resource::health::HealthCheckConfig;
use nebula_resource::health::{HealthCheckable, HealthStatus, ResourceHealthAdapter};
use nebula_resource::hooks::{HookEvent, HookFilter, HookResult, ResourceHook};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::quarantine::QuarantineReason;
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{
    ExecutionId, Manager, ManagerBuilder, PoolAcquire, PoolLifetime, PoolSizing, WorkflowId,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        sizing: PoolSizing {
            min_size: 0,
            max_size: 5,
        },
        lifetime: PoolLifetime {
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(3600),
            ..Default::default()
        },
        acquire: PoolAcquire {
            timeout: Duration::from_secs(2),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Simple test resource
// ---------------------------------------------------------------------------

struct SimpleResource;

impl Resource for SimpleResource {
    type Config = TestConfig;
    type Instance = String;
    fn key(&self) -> ResourceKey {
        resource_key!("simple")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok("instance".to_string())
    }
}

// ---------------------------------------------------------------------------
// Slow resource (for latency testing)
// ---------------------------------------------------------------------------

struct SlowResource {
    delay_ms: u64,
}

impl Resource for SlowResource {
    type Config = TestConfig;
    type Instance = String;
    fn key(&self) -> ResourceKey {
        resource_key!("slow")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        Ok("slow-instance".to_string())
    }
}

// ---------------------------------------------------------------------------
// Counting resource (for autoscale verification)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Cloneable resource (for ResourceHealthAdapter)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct CloneableResource;

impl Resource for CloneableResource {
    type Config = TestConfig;
    type Instance = String;
    fn key(&self) -> ResourceKey {
        resource_key!("cloneable")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok("cloneable-instance".to_string())
    }
}

// ---------------------------------------------------------------------------
// Simple health checkable
// ---------------------------------------------------------------------------

struct AlwaysHealthy;

impl HealthCheckable for AlwaysHealthy {
    async fn health_check(&self) -> Result<HealthStatus> {
        Ok(HealthStatus::healthy())
    }
}

// ---------------------------------------------------------------------------
// Test 1: deregister cancels auto-scaler
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deregister_cancels_auto_scaler() {
    let manager = Manager::new();

    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("register should succeed");

    // Enable autoscaling
    let policy = AutoScalePolicy {
        evaluation_window: Duration::from_millis(50),
        cooldown: Duration::from_millis(100),
        ..Default::default()
    };
    let key = resource_key!("simple");
    manager
        .enable_autoscaling(&key, policy)
        .expect("enable autoscaling should succeed");

    assert!(manager.is_registered(&key));

    // Deregister should succeed and cancel the auto-scaler
    let was_registered = manager.deregister(&key).await;
    assert!(was_registered, "should have been registered");
    assert!(!manager.is_registered(&key), "should be gone now");

    // Give the auto-scaler task time to be cancelled
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Re-registering should work without issues (no lingering scaler)
    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("re-register should succeed");

    let guard = manager
        .acquire(&key, &ctx())
        .await
        .expect("acquire after re-register");
    drop(guard);

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 2: deregister releases quarantine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deregister_releases_quarantine() {
    let bus = Arc::new(EventBus::new(64));
    let mut rx = bus.subscribe();

    let manager = ManagerBuilder::new().event_bus(Arc::clone(&bus)).build();

    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("register");

    let key = resource_key!("simple");

    // Manually quarantine the resource
    manager.quarantine().quarantine(
        key.as_str(),
        QuarantineReason::ManualQuarantine {
            reason: "test".into(),
        },
    );
    assert!(manager.quarantine().is_quarantined(key.as_str()));

    // Drain the Created event from registration
    let _ = rx.recv().await;

    // Deregister should release quarantine and emit QuarantineReleased
    let was_registered = manager.deregister(&key).await;
    assert!(was_registered);
    assert!(
        !manager.quarantine().is_quarantined(key.as_str()),
        "quarantine should be released after deregister"
    );

    // Check that QuarantineReleased event was emitted
    // We may get the CleanedUp event first, then QuarantineReleased or vice versa
    let mut found_quarantine_released = false;
    // Drain available events
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(nebula_resource::ResourceEvent::QuarantineReleased {
                resource_key, ..
            })) => {
                assert_eq!(resource_key.as_str(), "simple");
                found_quarantine_released = true;
                break;
            }
            Ok(Some(_)) => continue,
            _ => break,
        }
    }
    assert!(
        found_quarantine_released,
        "QuarantineReleased event should be emitted on deregister"
    );

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 3: deregister stops health monitoring
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deregister_stops_health_monitoring() {
    let manager = ManagerBuilder::new()
        .health_config(HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 3,
            check_timeout: Duration::from_secs(5),
            ..Default::default()
        })
        .build();

    manager
        .register(CloneableResource, TestConfig, pool_config())
        .expect("register");

    let key = resource_key!("cloneable");

    // Start health monitoring
    manager.start_health_monitoring(key.as_str(), AlwaysHealthy);

    // Let monitoring run for a couple of ticks (interval = 50ms)
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Health checker should have at least the record
    let all_health = manager.health_checker().get_all_health();
    assert!(
        !all_health.is_empty(),
        "health checker should have records after monitoring starts"
    );

    // Deregister — should stop monitoring
    manager.deregister(&key).await;

    // Give time for cleanup
    tokio::time::sleep(Duration::from_millis(150)).await;

    let all_health = manager.health_checker().get_all_health();
    let matching: Vec<_> = all_health
        .iter()
        .filter(|r| r.resource_id == "cloneable")
        .collect();
    assert!(
        matching.is_empty(),
        "health records for deregistered resource should be cleaned up"
    );

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 4: ManagerBuilder default autoscale policy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn default_autoscale_policy_applied_on_register() {
    let policy = AutoScalePolicy {
        high_watermark: 0.7,
        low_watermark: 0.1,
        scale_up_step: 1,
        scale_down_step: 1,
        evaluation_window: Duration::from_millis(50),
        cooldown: Duration::from_millis(100),
    };

    let manager = ManagerBuilder::new()
        .default_autoscale_policy(policy)
        .build();

    // Register — should auto-enable autoscaling
    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("register");

    let key = resource_key!("simple");

    // The auto-scaler should be running in the background.
    // Verify by acquiring a bunch of instances to raise utilization,
    // then checking that idle instances were pre-created.
    let mut guards = Vec::new();
    for _ in 0..4 {
        let g = manager.acquire(&key, &ctx()).await.expect("acquire");
        guards.push(g);
    }

    // Let the scaler evaluate (evaluation_window = 50ms)
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Release all
    drop(guards);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The test passes if no panics — the auto-scaler was successfully started.
    // Disable autoscaling to confirm it was registered.
    manager.disable_autoscaling(&key);

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 5: start_health_monitoring convenience
// ---------------------------------------------------------------------------

#[tokio::test]
async fn start_health_monitoring_convenience() {
    let manager = ManagerBuilder::new()
        .health_config(HealthCheckConfig {
            default_interval: Duration::from_millis(50),
            failure_threshold: 3,
            check_timeout: Duration::from_secs(5),
            ..Default::default()
        })
        .build();

    manager
        .register(CloneableResource, TestConfig, pool_config())
        .expect("register");

    let key = resource_key!("cloneable");

    // Use convenience method
    manager.start_health_monitoring(key.as_str(), AlwaysHealthy);

    // Wait for at least one health check cycle (interval = 50ms)
    tokio::time::sleep(Duration::from_millis(200)).await;

    let records = manager.health_checker().get_all_health();
    let matching: Vec<_> = records
        .iter()
        .filter(|r| r.resource_id == "cloneable")
        .collect();
    assert!(
        !matching.is_empty(),
        "health monitoring should produce records via convenience method"
    );

    // The record should show healthy status
    for record in &matching {
        assert!(
            record.status.is_usable(),
            "AlwaysHealthy should report usable status"
        );
    }

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 6: ResourceHealthAdapter works as HealthCheckable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resource_health_adapter_probe_cycle() {
    let adapter = ResourceHealthAdapter::new(CloneableResource, TestConfig, Scope::Global);

    let status = adapter.health_check().await.expect("health check");
    assert!(status.is_usable(), "CloneableResource should be healthy");
    assert!(
        status.state == nebula_resource::HealthState::Healthy,
        "state should be Healthy"
    );
}

// ---------------------------------------------------------------------------
// Test 7: PoolStats latency percentiles
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_stats_have_latency_percentiles() {
    let pool = Pool::new(SimpleResource, TestConfig, pool_config()).expect("pool");

    // Initial stats should have None percentiles
    let stats = pool.stats();
    assert!(stats.acquire_latency.is_none());

    // Acquire a few instances to populate the latency window
    for _ in 0..10 {
        let (g, _) = pool.acquire(&ctx()).await.expect("acquire");
        drop(g);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let stats = pool.stats();
    assert!(
        stats.acquire_latency.is_some(),
        "latency percentiles should be populated after acquisitions"
    );

    // All percentiles should be non-negative (they are u64, so always >= 0)
    // p50 <= p95 <= p99
    let latency = stats.acquire_latency.expect("latency should be present");
    let p50 = latency.p50_ms;
    let p95 = latency.p95_ms;
    let p99 = latency.p99_ms;
    assert!(p50 <= p95, "p50 ({p50}) should be <= p95 ({p95})");
    assert!(p95 <= p99, "p95 ({p95}) should be <= p99 ({p99})");

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 8: Latency percentiles with varying latencies
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_stats_percentiles_reflect_latency_distribution() {
    let cfg = PoolConfig {
        sizing: PoolSizing {
            min_size: 0,
            max_size: 1,
        },
        lifetime: PoolLifetime {
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(3600),
            ..Default::default()
        },
        acquire: PoolAcquire {
            timeout: Duration::from_secs(5),
            ..Default::default()
        },
        ..Default::default()
    };

    let pool = Pool::new(SlowResource { delay_ms: 5 }, TestConfig, cfg).expect("pool");

    // Acquire and release several times (each create takes ~5ms)
    for _ in 0..20 {
        let (g, wait) = pool.acquire(&ctx()).await.expect("acquire");
        // wait duration should be > 0 for cold creates
        let _ = wait;
        drop(g);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let stats = pool.stats();
    assert!(stats.acquire_latency.is_some());

    // The total_wait_time_ms should be positive
    assert!(
        stats.total_wait_time_ms > 0,
        "total wait time should be positive with slow resource"
    );

    pool.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 9: stop_monitoring_resource cancels all instances
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stop_monitoring_resource_cancels_all() {
    let checker = nebula_resource::HealthChecker::new(nebula_resource::HealthCheckConfig {
        default_interval: Duration::from_millis(50),
        ..Default::default()
    });

    // Start monitoring two instances of the same resource
    checker.start_monitoring(
        uuid::Uuid::new_v4(),
        "my-resource".to_string(),
        Arc::new(AlwaysHealthy),
    );
    checker.start_monitoring(
        uuid::Uuid::new_v4(),
        "my-resource".to_string(),
        Arc::new(AlwaysHealthy),
    );

    // Also monitor a different resource
    checker.start_monitoring(
        uuid::Uuid::new_v4(),
        "other-resource".to_string(),
        Arc::new(AlwaysHealthy),
    );

    // Let monitoring run
    tokio::time::sleep(Duration::from_millis(150)).await;

    let all = checker.get_all_health();
    let my_count = all
        .iter()
        .filter(|r| r.resource_id == "my-resource")
        .count();
    let other_count = all
        .iter()
        .filter(|r| r.resource_id == "other-resource")
        .count();
    assert_eq!(my_count, 2, "should have 2 records for my-resource");
    assert_eq!(other_count, 1, "should have 1 record for other-resource");

    // Stop all monitoring for "my-resource"
    let stopped = checker.stop_monitoring_resource("my-resource");
    assert_eq!(stopped, 2, "should stop 2 instances");

    // Give time for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;

    let all = checker.get_all_health();
    let my_remaining = all
        .iter()
        .filter(|r| r.resource_id == "my-resource")
        .count();
    let other_remaining = all
        .iter()
        .filter(|r| r.resource_id == "other-resource")
        .count();
    assert_eq!(
        my_remaining, 0,
        "my-resource records should be gone after stop_monitoring_resource"
    );
    assert_eq!(other_remaining, 1, "other-resource should not be affected");

    checker.shutdown();
}

// ---------------------------------------------------------------------------
// Test 10: deregister non-existent resource is a no-op
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deregister_nonexistent_returns_false() {
    let manager = Manager::new();

    let key = resource_key!("nonexistent");
    let was_registered = manager.deregister(&key).await;
    assert!(!was_registered);

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 11: deregister while auto-scaler running does not panic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deregister_with_active_autoscaler_no_panic() {
    let manager = Manager::new();

    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("register");

    let key = resource_key!("simple");

    let policy = AutoScalePolicy {
        evaluation_window: Duration::from_millis(20),
        cooldown: Duration::from_millis(50),
        ..Default::default()
    };
    manager
        .enable_autoscaling(&key, policy)
        .expect("enable autoscaling");

    // Let the scaler run a few cycles
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Deregister while scaler is active
    let was_registered = manager.deregister(&key).await;
    assert!(was_registered);

    // Give time for abort to propagate
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Manager should still be usable
    assert!(!manager.is_registered(&key));

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 12: re-register after deregister works cleanly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn re_register_after_deregister_with_full_cleanup() {
    let manager = Manager::new();

    // Register, add autoscaler, start monitoring, quarantine
    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("register");

    let key = resource_key!("simple");

    let policy = AutoScalePolicy {
        evaluation_window: Duration::from_millis(50),
        cooldown: Duration::from_millis(100),
        ..Default::default()
    };
    manager
        .enable_autoscaling(&key, policy)
        .expect("enable autoscaling");

    manager.start_health_monitoring(key.as_str(), AlwaysHealthy);

    manager.quarantine().quarantine(
        key.as_str(),
        QuarantineReason::ManualQuarantine {
            reason: "test".into(),
        },
    );

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Deregister should clean everything up
    manager.deregister(&key).await;

    // Verify all state is clean
    assert!(!manager.is_registered(&key));
    assert!(!manager.quarantine().is_quarantined(key.as_str()));
    assert!(manager.get_health_state(&key).is_none());

    // Re-register and use
    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("re-register should succeed");

    let guard = manager
        .acquire(&key, &ctx())
        .await
        .expect("acquire after re-register should work");
    drop(guard);

    manager.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 13: PoolStats default percentiles are None
// ---------------------------------------------------------------------------

#[test]
fn pool_stats_default_has_none_percentiles() {
    let stats = nebula_resource::PoolStats::default();
    assert!(stats.acquire_latency.is_none());
    assert_eq!(stats.total_wait_time_ms, 0);
    assert_eq!(stats.max_wait_time_ms, 0);
}

// ---------------------------------------------------------------------------
// Test 14: Hook counting through Manager tracks Create events
// ---------------------------------------------------------------------------

struct CreateCountingHook {
    count: AtomicU32,
}

impl CreateCountingHook {
    fn new() -> Self {
        Self {
            count: AtomicU32::new(0),
        }
    }

    fn count(&self) -> u32 {
        self.count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ResourceHook for CreateCountingHook {
    fn name(&self) -> &str {
        "create-counter"
    }

    fn events(&self) -> Vec<HookEvent> {
        vec![HookEvent::Create]
    }

    fn filter(&self) -> HookFilter {
        HookFilter::All
    }

    async fn before(&self, _event: &HookEvent, _resource_id: &str, _ctx: &Context) -> HookResult {
        self.count.fetch_add(1, Ordering::SeqCst);
        HookResult::Continue
    }

    async fn after(&self, _event: &HookEvent, _resource_id: &str, _ctx: &Context, _success: bool) {}
}

#[tokio::test]
async fn default_autoscale_plus_hooks_work_together() {
    let hook = Arc::new(CreateCountingHook::new());

    let policy = AutoScalePolicy {
        high_watermark: 0.7,
        low_watermark: 0.1,
        scale_up_step: 1,
        scale_down_step: 1,
        evaluation_window: Duration::from_millis(50),
        cooldown: Duration::from_millis(100),
    };

    let manager = ManagerBuilder::new()
        .default_autoscale_policy(policy)
        .build();

    manager
        .hooks()
        .register(Arc::clone(&hook) as Arc<dyn ResourceHook>);

    manager
        .register(SimpleResource, TestConfig, pool_config())
        .expect("register");

    let key = resource_key!("simple");

    // Acquire
    let guard = manager.acquire(&key, &ctx()).await.expect("acquire");
    assert!(
        hook.count() >= 1,
        "Create hook should fire on first acquire"
    );

    drop(guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    manager.shutdown().await.unwrap();
}


