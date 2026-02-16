//! Integration tests for EventBus, health state transition events,
//! and pool exhaustion events.

use std::sync::Arc;
use std::time::Duration;

use nebula_resource::events::{EventBus, ResourceEvent};
use nebula_resource::health::{
    HealthCheckConfig, HealthCheckable, HealthChecker, HealthState, HealthStatus,
};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::prelude::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig {
    value: String,
}

impl Config for TestConfig {
    fn validate(&self) -> Result<()> {
        Ok(())
    }
}

struct TestResource;

impl Resource for TestResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        "test"
    }

    async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        Ok(format!("inst-{}", config.value))
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

fn test_config() -> TestConfig {
    TestConfig {
        value: "hello".into(),
    }
}

// ---------------------------------------------------------------------------
// T024: EventBus emits ResourceEvent variants and subscribers receive them
// ---------------------------------------------------------------------------

#[tokio::test]
async fn event_bus_created_event_received_by_subscriber() {
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();

    bus.emit(ResourceEvent::Created {
        resource_id: "db".to_string(),
        scope: Scope::Global,
    });

    let event = rx.recv().await.expect("should receive Created event");
    match event {
        ResourceEvent::Created { resource_id, scope } => {
            assert_eq!(resource_id, "db");
            assert_eq!(scope, Scope::Global);
        }
        other => panic!("expected Created, got {other:?}"),
    }
}

#[tokio::test]
async fn event_bus_multiple_event_types_received_in_order() {
    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();

    bus.emit(ResourceEvent::Created {
        resource_id: "r1".to_string(),
        scope: Scope::Global,
    });
    bus.emit(ResourceEvent::Acquired {
        resource_id: "r1".to_string(),
        pool_stats: nebula_resource::pool::PoolStats::default(),
    });
    bus.emit(ResourceEvent::Released {
        resource_id: "r1".to_string(),
        usage_duration: Duration::from_millis(100),
    });
    bus.emit(ResourceEvent::Error {
        resource_id: "r1".to_string(),
        error: "boom".to_string(),
    });

    assert!(matches!(
        rx.recv().await.unwrap(),
        ResourceEvent::Created { .. }
    ));
    assert!(matches!(
        rx.recv().await.unwrap(),
        ResourceEvent::Acquired { .. }
    ));
    assert!(matches!(
        rx.recv().await.unwrap(),
        ResourceEvent::Released { .. }
    ));
    assert!(matches!(
        rx.recv().await.unwrap(),
        ResourceEvent::Error { .. }
    ));
}

#[tokio::test]
async fn event_bus_multiple_subscribers_all_receive() {
    let bus = EventBus::new(64);
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();
    let mut rx3 = bus.subscribe();

    bus.emit(ResourceEvent::Created {
        resource_id: "db".to_string(),
        scope: Scope::Global,
    });

    assert!(matches!(
        rx1.recv().await.unwrap(),
        ResourceEvent::Created { .. }
    ));
    assert!(matches!(
        rx2.recv().await.unwrap(),
        ResourceEvent::Created { .. }
    ));
    assert!(matches!(
        rx3.recv().await.unwrap(),
        ResourceEvent::Created { .. }
    ));
}

#[tokio::test]
async fn manager_register_emits_created_event() {
    let bus = Arc::new(EventBus::new(64));
    let mgr = Manager::with_event_bus(Arc::clone(&bus));
    let mut rx = bus.subscribe();

    mgr.register(TestResource, test_config(), PoolConfig::default())
        .unwrap();

    let event = rx.recv().await.expect("should receive Created event");
    match event {
        ResourceEvent::Created { resource_id, scope } => {
            assert_eq!(resource_id, "test");
            assert_eq!(scope, Scope::Global);
        }
        other => panic!("expected Created, got {other:?}"),
    }
}

#[tokio::test]
async fn manager_acquire_emits_acquired_event() {
    let bus = Arc::new(EventBus::new(64));
    let mgr = Manager::with_event_bus(Arc::clone(&bus));
    let mut rx = bus.subscribe();

    mgr.register(TestResource, test_config(), PoolConfig::default())
        .unwrap();

    // Drain the Created event
    let _ = rx.recv().await.unwrap();

    let _guard = mgr.acquire("test", &ctx()).await.unwrap();

    let event = rx.recv().await.expect("should receive Acquired event");
    assert!(matches!(event, ResourceEvent::Acquired { resource_id, .. } if resource_id == "test"));
}

#[tokio::test]
async fn manager_acquire_failure_emits_error_event() {
    let bus = Arc::new(EventBus::new(64));
    let mgr = Manager::with_event_bus(Arc::clone(&bus));
    let mut rx = bus.subscribe();

    // Try to acquire a nonexistent resource
    let result = mgr.acquire("nonexistent", &ctx()).await;
    assert!(result.is_err());

    let event = rx.recv().await.expect("should receive Error event");
    assert!(
        matches!(event, ResourceEvent::Error { resource_id, .. } if resource_id == "nonexistent")
    );
}

// ---------------------------------------------------------------------------
// T026: Health state transition emits HealthChanged event
// ---------------------------------------------------------------------------

struct FlippableHealthCheck {
    should_fail: Arc<std::sync::atomic::AtomicBool>,
}

impl HealthCheckable for FlippableHealthCheck {
    async fn health_check(&self) -> nebula_resource::error::Result<HealthStatus> {
        if self.should_fail.load(std::sync::atomic::Ordering::Relaxed) {
            Ok(HealthStatus::unhealthy("failing"))
        } else {
            Ok(HealthStatus::healthy())
        }
    }
}

#[tokio::test]
async fn health_state_transition_emits_health_changed_event() {
    let bus = Arc::new(EventBus::new(64));
    let mut rx = bus.subscribe();

    let config = HealthCheckConfig {
        default_interval: Duration::from_millis(50),
        failure_threshold: 10,
        check_timeout: Duration::from_secs(1),
    };
    let checker = HealthChecker::with_event_bus(config, Arc::clone(&bus));

    let should_fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let instance = Arc::new(FlippableHealthCheck {
        should_fail: Arc::clone(&should_fail),
    });

    let instance_id = uuid::Uuid::new_v4();
    checker.start_monitoring(instance_id, "test-res".to_string(), instance);

    // Wait for first check (healthy) — should emit HealthChanged(Unknown -> Healthy)
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out")
        .expect("should receive HealthChanged");

    match event {
        ResourceEvent::HealthChanged {
            resource_id,
            from,
            to,
        } => {
            assert_eq!(resource_id, "test-res");
            assert_eq!(from, HealthState::Unknown);
            assert_eq!(to, HealthState::Healthy);
        }
        other => panic!("expected HealthChanged, got {other:?}"),
    }

    // Now flip to unhealthy
    should_fail.store(true, std::sync::atomic::Ordering::Relaxed);

    // Wait for the transition event
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out")
        .expect("should receive HealthChanged for transition to unhealthy");

    match event {
        ResourceEvent::HealthChanged {
            resource_id,
            from,
            to,
        } => {
            assert_eq!(resource_id, "test-res");
            assert_eq!(from, HealthState::Healthy);
            assert!(matches!(to, HealthState::Unhealthy { .. }));
        }
        other => panic!("expected HealthChanged(Healthy->Unhealthy), got {other:?}"),
    }

    checker.shutdown();
}

#[tokio::test]
async fn no_health_changed_event_when_state_unchanged() {
    let bus = Arc::new(EventBus::new(64));
    let mut rx = bus.subscribe();

    let config = HealthCheckConfig {
        default_interval: Duration::from_millis(50),
        failure_threshold: 10,
        check_timeout: Duration::from_secs(1),
    };
    let checker = HealthChecker::with_event_bus(config, Arc::clone(&bus));

    let instance = Arc::new(FlippableHealthCheck {
        should_fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });

    let instance_id = uuid::Uuid::new_v4();
    checker.start_monitoring(instance_id, "test-res".to_string(), instance);

    // First check: Unknown -> Healthy
    let _ = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out")
        .expect("should receive initial HealthChanged");

    // Wait for a couple more checks (state stays Healthy)
    tokio::time::sleep(Duration::from_millis(150)).await;

    // There should be no more HealthChanged events in the channel
    let result = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await;
    assert!(
        result.is_err(),
        "should not receive HealthChanged when state is unchanged"
    );

    checker.shutdown();
}

// ---------------------------------------------------------------------------
// T027: Pool exhaustion emits PoolExhausted event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_exhaustion_emits_pool_exhausted_event() {
    let bus = Arc::new(EventBus::new(64));
    let mut rx = bus.subscribe();

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 1,
        acquire_timeout: Duration::from_millis(100),
        ..Default::default()
    };
    let pool = Pool::with_event_bus(
        TestResource,
        test_config(),
        pool_config,
        Some(Arc::clone(&bus)),
    )
    .unwrap();

    // Hold the only permit
    let _guard = pool.acquire(&ctx()).await.unwrap();

    // Second acquire should fail with PoolExhausted
    let result = pool.acquire(&ctx()).await;
    assert!(result.is_err());

    // Check that PoolExhausted event was emitted
    let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out")
        .expect("should receive PoolExhausted event");

    match event {
        ResourceEvent::PoolExhausted { resource_id, .. } => {
            assert_eq!(resource_id, "test");
        }
        other => panic!("expected PoolExhausted, got {other:?}"),
    }
}

#[tokio::test]
async fn pool_shutdown_emits_cleaned_up_shutdown_events() {
    let bus = Arc::new(EventBus::new(64));
    let mut rx = bus.subscribe();

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::with_event_bus(
        TestResource,
        test_config(),
        pool_config,
        Some(Arc::clone(&bus)),
    )
    .unwrap();

    // Acquire and return to create an idle instance
    {
        let _guard = pool.acquire(&ctx()).await.unwrap();
    }
    // Wait for the guard's spawned drop task to return the instance to idle
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Shutdown should clean up idle instances
    pool.shutdown().await.unwrap();

    // Also wait for any in-flight guard drop tasks post-shutdown
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Collect events — should find at least one CleanedUp(Shutdown)
    let mut found_shutdown_cleanup = false;
    // Drain all events with a short timeout
    loop {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(ResourceEvent::CleanedUp { reason, .. })) => {
                if matches!(reason, nebula_resource::events::CleanupReason::Shutdown) {
                    found_shutdown_cleanup = true;
                }
            }
            Ok(Ok(_)) => continue, // skip non-CleanedUp events (e.g. Released)
            _ => break,
        }
    }
    assert!(
        found_shutdown_cleanup,
        "expected at least one CleanedUp(Shutdown) event"
    );
}

#[tokio::test]
async fn pool_guard_drop_emits_released_event() {
    let bus = Arc::new(EventBus::new(64));
    let mut rx = bus.subscribe();

    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 2,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::with_event_bus(
        TestResource,
        test_config(),
        pool_config,
        Some(Arc::clone(&bus)),
    )
    .unwrap();

    {
        let _guard = pool.acquire(&ctx()).await.unwrap();
        // Hold for a bit to get a measurable duration
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Wait for the spawned drop task
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Should find a Released event
    let mut found_released = false;
    loop {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(ResourceEvent::Released {
                resource_id,
                usage_duration,
            })) => {
                assert_eq!(resource_id, "test");
                assert!(usage_duration >= Duration::from_millis(10));
                found_released = true;
                break;
            }
            Ok(Ok(_)) => continue, // skip other events
            _ => break,
        }
    }
    assert!(found_released, "expected Released event after guard drop");
}
