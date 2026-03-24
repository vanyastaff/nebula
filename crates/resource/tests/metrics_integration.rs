//! Integration test for MetricsCollector processing events from EventBus.
//!
//! Verifies that the collector subscribes, processes events without panics,
//! and terminates cleanly when the bus is dropped.

use std::sync::Arc;
use std::time::Duration;

use nebula_resource::events::{CleanupReason, EventBus, ResourceEvent};
use nebula_resource::metrics::MetricsCollector;

use nebula_resource::scope::Scope;

#[tokio::test]
async fn metrics_collector_processes_all_event_types() {
    let bus = Arc::new(EventBus::new(64));
    let collector = MetricsCollector::new(&bus);
    let cancel = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(collector.run(cancel));

    // Emit one of each event type
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

    // Give collector time to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drop bus to close the channel, which terminates the collector
    drop(bus);

    // Collector should terminate cleanly
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "collector should terminate after bus is dropped"
    );
}

#[tokio::test]
async fn metrics_collector_terminates_when_bus_dropped() {
    let bus = Arc::new(EventBus::new(16));
    let collector = MetricsCollector::new(&bus);
    let cancel = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(collector.run(cancel));

    // Drop the bus immediately
    drop(bus);

    // Collector should terminate
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "collector should terminate when bus is dropped"
    );
}

#[tokio::test]
async fn spawn_metrics_collector_helper_works() {
    let bus = Arc::new(EventBus::new(16));
    let cancel = tokio_util::sync::CancellationToken::new();
    let handle = nebula_resource::metrics::spawn_metrics_collector(&bus, cancel);

    let key = nebula_core::resource_key!("x");
    bus.emit(ResourceEvent::Created {
        resource_key: key,
        scope: Scope::Global,
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(bus);

    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(result.is_ok(), "spawned collector should terminate cleanly");
}


