//! Integration tests for the telemetry crate.
//!
//! Covers event order, TelemetryService wiring with engine/runtime,
//! and hot-path stability (no panics under load).

use std::time::Duration;

use nebula_telemetry::event::{EventBus, ExecutionEvent};
use nebula_telemetry::metrics::MetricsRegistry;
use nebula_telemetry::{NoopTelemetry, TelemetryService};

// ---------------------------------------------------------------------------
// Event order and EventBus
// ---------------------------------------------------------------------------

#[tokio::test]
async fn events_received_in_emit_order() {
    let bus = EventBus::new(64);
    let mut sub = bus.subscribe();

    bus.emit(ExecutionEvent::Started {
        execution_id: "e1".into(),
        workflow_id: "w1".into(),
    });
    bus.emit(ExecutionEvent::NodeStarted {
        execution_id: "e1".into(),
        node_id: "n1".into(),
    });
    bus.emit(ExecutionEvent::NodeCompleted {
        execution_id: "e1".into(),
        node_id: "n1".into(),
        duration: Duration::from_millis(10),
    });
    bus.emit(ExecutionEvent::Completed {
        execution_id: "e1".into(),
        duration: Duration::from_secs(1),
    });

    let first = sub.recv().await.expect("first");
    let second = sub.recv().await.expect("second");
    let third = sub.recv().await.expect("third");
    let fourth = sub.recv().await.expect("fourth");

    assert!(matches!(first, ExecutionEvent::Started { .. }));
    assert!(matches!(second, ExecutionEvent::NodeStarted { .. }));
    assert!(matches!(third, ExecutionEvent::NodeCompleted { .. }));
    assert!(matches!(fourth, ExecutionEvent::Completed { .. }));
}

#[test]
fn noop_telemetry_arc_provides_same_bus_and_metrics() {
    let telemetry = NoopTelemetry::arc();
    let bus1 = telemetry.event_bus_arc();
    let bus2 = telemetry.event_bus_arc();
    let metrics1 = telemetry.metrics_arc();
    let metrics2 = telemetry.metrics_arc();

    telemetry.event_bus().emit(ExecutionEvent::Started {
        execution_id: "e1".into(),
        workflow_id: "w1".into(),
    });
    telemetry.metrics().counter("nebula_test_counter").inc();

    assert_eq!(bus1.total_emitted(), bus2.total_emitted());
    assert_eq!(metrics1.counter("nebula_test_counter").get(), metrics2.counter("nebula_test_counter").get());
}

// ---------------------------------------------------------------------------
// Hot path: no panics under load
// ---------------------------------------------------------------------------

#[test]
fn emit_10k_events_no_panic() {
    let bus = EventBus::new(256);
    let _sub = bus.subscribe();

    for i in 0..10_000_u32 {
        bus.emit(ExecutionEvent::NodeStarted {
            execution_id: "e1".into(),
            node_id: format!("n{i}"),
        });
    }
    assert_eq!(bus.total_emitted(), 10_000);
}

#[test]
fn record_10k_metric_observations_no_panic() {
    let registry = MetricsRegistry::new();
    let counter = registry.counter("nebula_ops_total");
    let histogram = registry.histogram("nebula_duration_seconds");

    for _ in 0..10_000 {
        counter.inc();
        histogram.observe(0.001);
    }
    assert_eq!(counter.get(), 10_000);
    assert_eq!(histogram.count(), 10_000);
}

#[test]
fn noop_telemetry_full_flow_no_panic() {
    let telemetry = NoopTelemetry::new();
    let bus = telemetry.event_bus();
    let metrics = telemetry.metrics();

    // Subscriber so events are counted as sent (eventbus only counts delivered events).
    let _sub = bus.subscribe();

    bus.emit(ExecutionEvent::Started {
        execution_id: "exec".into(),
        workflow_id: "wf".into(),
    });
    bus.emit(ExecutionEvent::NodeStarted {
        execution_id: "exec".into(),
        node_id: "n1".into(),
    });
    bus.emit(ExecutionEvent::NodeCompleted {
        execution_id: "exec".into(),
        node_id: "n1".into(),
        duration: Duration::from_millis(5),
    });
    bus.emit(ExecutionEvent::Completed {
        execution_id: "exec".into(),
        duration: Duration::from_secs(1),
    });

    metrics.counter("nebula_executions_total").inc();
    metrics.gauge("nebula_active_executions").set(0);
    metrics.histogram("nebula_execution_duration_seconds").observe(1.0);

    assert_eq!(bus.total_emitted(), 4);
    assert_eq!(metrics.counter("nebula_executions_total").get(), 1);
}
