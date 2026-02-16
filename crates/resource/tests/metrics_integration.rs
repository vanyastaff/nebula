//! Integration test for MetricsCollector processing events from EventBus.
//!
//! Verifies that the collector subscribes, processes events without panics,
//! and terminates cleanly when the bus is dropped.

#![cfg(feature = "metrics")]

use std::sync::Arc;
use std::time::Duration;

use nebula_resource::events::{CleanupReason, EventBus, ResourceEvent};
use nebula_resource::metrics::MetricsCollector;
use nebula_resource::pool::PoolStats;
use nebula_resource::scope::Scope;

#[tokio::test]
async fn metrics_collector_processes_all_event_types() {
    let bus = Arc::new(EventBus::new(64));
    let collector = MetricsCollector::new(&bus);

    let handle = tokio::spawn(collector.run());

    // Emit one of each event type
    bus.emit(ResourceEvent::Created {
        resource_id: "db".to_string(),
        scope: Scope::Global,
    });
    bus.emit(ResourceEvent::Acquired {
        resource_id: "db".to_string(),
        pool_stats: PoolStats::default(),
    });
    bus.emit(ResourceEvent::Released {
        resource_id: "db".to_string(),
        usage_duration: Duration::from_millis(42),
    });
    bus.emit(ResourceEvent::CleanedUp {
        resource_id: "db".to_string(),
        reason: CleanupReason::IdleTimeout,
    });
    bus.emit(ResourceEvent::Error {
        resource_id: "db".to_string(),
        error: "test error".to_string(),
    });
    bus.emit(ResourceEvent::PoolExhausted {
        resource_id: "db".to_string(),
        waiters: 3,
    });
    bus.emit(ResourceEvent::HealthChanged {
        resource_id: "db".to_string(),
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

    let handle = tokio::spawn(collector.run());

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
    let handle = nebula_resource::metrics::spawn_metrics_collector(&bus);

    bus.emit(ResourceEvent::Created {
        resource_id: "x".to_string(),
        scope: Scope::Global,
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(bus);

    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(result.is_ok(), "spawned collector should terminate cleanly");
}
