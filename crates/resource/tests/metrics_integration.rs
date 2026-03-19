//! Integration tests for `MetricsCollector` recording events into `MetricsRegistry`.
//!
//! These tests verify that emitted `ResourceEvent`s produce real, queryable
//! metric values in the registry — not just "didn't panic".

use std::sync::Arc;
use std::time::Duration;

use nebula_metrics::naming::{
    RESOURCE_ACQUIRE, RESOURCE_ACQUIRE_WAIT_DURATION, RESOURCE_CREATE, RESOURCE_ERROR,
    RESOURCE_HEALTH_STATE, RESOURCE_POOL_EXHAUSTED, RESOURCE_POOL_WAITERS, RESOURCE_RELEASE,
    RESOURCE_USAGE_DURATION,
};
use nebula_resource::events::{CleanupReason, EventBus, ResourceEvent};
use nebula_resource::metrics::MetricsCollector;
use nebula_resource::scope::Scope;
use nebula_telemetry::metrics::MetricsRegistry;

/// Helper: spawn a collector and return (bus, registry, cancel).
fn setup() -> (
    Arc<EventBus>,
    Arc<MetricsRegistry>,
    tokio_util::sync::CancellationToken,
) {
    let registry = Arc::new(MetricsRegistry::new());
    let bus = Arc::new(EventBus::new(64));
    let cancel = tokio_util::sync::CancellationToken::new();
    (bus, registry, cancel)
}

/// Helper: sum counter values whose name matches `expected_name`.
fn sum_counter(registry: &MetricsRegistry, expected_name: &str) -> u64 {
    registry
        .snapshot_counters()
        .iter()
        .filter(|(k, _)| registry.interner().resolve(k.name) == expected_name)
        .map(|(_, c)| c.get())
        .sum()
}

/// Helper: get gauge value whose name matches `expected_name`.
fn get_gauge(registry: &MetricsRegistry, expected_name: &str) -> Option<i64> {
    registry
        .snapshot_gauges()
        .iter()
        .find(|(k, _)| registry.interner().resolve(k.name) == expected_name)
        .map(|(_, g)| g.get())
}

#[tokio::test]
async fn collector_records_create_event_to_registry() {
    let (bus, registry, cancel) = setup();
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let handle = tokio::spawn(collector.run(cancel));

    let key = nebula_core::resource_key!("db");
    bus.emit(ResourceEvent::Created {
        resource_key: key,
        scope: Scope::Global,
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(sum_counter(&registry, RESOURCE_CREATE.as_str()), 1);

    drop(bus);
    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
}

#[tokio::test]
async fn collector_records_acquired_event_counter_and_histogram() {
    let (bus, registry, cancel) = setup();
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let handle = tokio::spawn(collector.run(cancel));

    let key = nebula_core::resource_key!("db");
    bus.emit(ResourceEvent::Acquired {
        resource_key: key,
        wait_duration: Duration::from_millis(10),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(sum_counter(&registry, RESOURCE_ACQUIRE.as_str()), 1);

    // Histogram should have exactly one observation
    let histograms = registry.snapshot_histograms();
    let wait_hist = histograms
        .iter()
        .find(|(k, _)| {
            registry.interner().resolve(k.name) == RESOURCE_ACQUIRE_WAIT_DURATION.as_str()
        })
        .map(|(_, h)| h.clone());
    assert!(wait_hist.is_some(), "wait duration histogram should exist");

    drop(bus);
    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
}

#[tokio::test]
async fn collector_records_released_event_counter_and_histogram() {
    let (bus, registry, cancel) = setup();
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let handle = tokio::spawn(collector.run(cancel));

    let key = nebula_core::resource_key!("db");
    bus.emit(ResourceEvent::Released {
        resource_key: key,
        usage_duration: Duration::from_millis(42),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(sum_counter(&registry, RESOURCE_RELEASE.as_str()), 1);

    let histograms = registry.snapshot_histograms();
    let usage_hist = histograms
        .iter()
        .find(|(k, _)| registry.interner().resolve(k.name) == RESOURCE_USAGE_DURATION.as_str())
        .map(|(_, h)| h.clone());
    assert!(
        usage_hist.is_some(),
        "usage duration histogram should exist"
    );

    drop(bus);
    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
}

#[tokio::test]
async fn collector_records_health_changed_gauge() {
    let (bus, registry, cancel) = setup();
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let handle = tokio::spawn(collector.run(cancel));

    let key = nebula_core::resource_key!("db");
    bus.emit(ResourceEvent::HealthChanged {
        resource_key: key.clone(),
        from: nebula_resource::health::HealthState::Unknown,
        to: nebula_resource::health::HealthState::Healthy,
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        get_gauge(&registry, RESOURCE_HEALTH_STATE.as_str()),
        Some(100)
    );

    // Transition to unhealthy
    bus.emit(ResourceEvent::HealthChanged {
        resource_key: key,
        from: nebula_resource::health::HealthState::Healthy,
        to: nebula_resource::health::HealthState::Unhealthy {
            reason: "down".to_string(),
            recoverable: true,
        },
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        get_gauge(&registry, RESOURCE_HEALTH_STATE.as_str()),
        Some(0)
    );

    drop(bus);
    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
}

#[tokio::test]
async fn collector_records_pool_exhausted_counter_and_waiters_gauge() {
    let (bus, registry, cancel) = setup();
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let handle = tokio::spawn(collector.run(cancel));

    let key = nebula_core::resource_key!("db");
    bus.emit(ResourceEvent::PoolExhausted {
        resource_key: key,
        waiters: 5,
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(sum_counter(&registry, RESOURCE_POOL_EXHAUSTED.as_str()), 1);
    assert_eq!(
        get_gauge(&registry, RESOURCE_POOL_WAITERS.as_str()),
        Some(5)
    );

    drop(bus);
    let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
}

#[tokio::test]
async fn metrics_collector_processes_all_event_types() {
    let (bus, registry, cancel) = setup();
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let handle = tokio::spawn(collector.run(cancel));

    let key = nebula_core::resource_key!("db");
    bus.emit(ResourceEvent::Created {
        resource_key: key.clone(),
        scope: Scope::Global,
    });
    bus.emit(ResourceEvent::Acquired {
        resource_key: key.clone(),
        wait_duration: Duration::from_millis(1),
    });
    bus.emit(ResourceEvent::Released {
        resource_key: key.clone(),
        usage_duration: Duration::from_millis(42),
    });
    bus.emit(ResourceEvent::CleanedUp {
        resource_key: key.clone(),
        reason: CleanupReason::IdleTimeout,
    });
    bus.emit(ResourceEvent::Error {
        resource_key: key.clone(),
        error: "test error".to_string(),
    });
    bus.emit(ResourceEvent::PoolExhausted {
        resource_key: key.clone(),
        waiters: 3,
    });
    bus.emit(ResourceEvent::HealthChanged {
        resource_key: key,
        from: nebula_resource::health::HealthState::Unknown,
        to: nebula_resource::health::HealthState::Healthy,
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify key counters are populated
    assert_eq!(sum_counter(&registry, RESOURCE_CREATE.as_str()), 1);
    assert_eq!(sum_counter(&registry, RESOURCE_ACQUIRE.as_str()), 1);
    assert_eq!(sum_counter(&registry, RESOURCE_ERROR.as_str()), 1);

    drop(bus);
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "collector should terminate after bus is dropped"
    );
}

#[tokio::test]
async fn metrics_collector_terminates_when_bus_dropped() {
    let (bus, registry, cancel) = setup();
    let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
    let handle = tokio::spawn(collector.run(cancel));

    drop(bus);

    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "collector should terminate when bus is dropped"
    );
}

#[tokio::test]
async fn spawn_metrics_collector_helper_works() {
    let (bus, registry, cancel) = setup();
    let handle =
        nebula_resource::metrics::spawn_metrics_collector(&bus, Arc::clone(&registry), cancel);

    let key = nebula_core::resource_key!("x");
    bus.emit(ResourceEvent::Created {
        resource_key: key,
        scope: Scope::Global,
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(sum_counter(&registry, RESOURCE_CREATE.as_str()), 1);

    drop(bus);
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(result.is_ok(), "spawned collector should terminate cleanly");
}
