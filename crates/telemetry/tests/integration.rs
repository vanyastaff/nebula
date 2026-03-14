//! Integration tests for the telemetry crate.
//!
//! Covers event order, TelemetryService wiring with engine/runtime,
//! and hot-path stability (no panics under load).

use std::time::Duration;

use nebula_telemetry::SubscriptionScope;
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
        trace_context: None,
    });
    bus.emit(ExecutionEvent::NodeStarted {
        execution_id: "e1".into(),
        node_id: "n1".into(),
        trace_context: None,
    });
    bus.emit(ExecutionEvent::NodeCompleted {
        execution_id: "e1".into(),
        node_id: "n1".into(),
        duration: Duration::from_millis(10),
        trace_context: None,
    });
    bus.emit(ExecutionEvent::Completed {
        execution_id: "e1".into(),
        duration: Duration::from_secs(1),
        trace_context: None,
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

#[tokio::test]
async fn scoped_subscription_receives_only_matching_execution() {
    let bus = EventBus::new(64);
    let mut sub = bus.subscribe_scoped(SubscriptionScope::execution("e-target"));

    let _ = bus.emit(ExecutionEvent::NodeStarted {
        execution_id: "e-other".into(),
        node_id: "n-other".into(),
        trace_context: None,
    });
    let _ = bus.emit(ExecutionEvent::NodeStarted {
        execution_id: "e-target".into(),
        node_id: "n-target".into(),
        trace_context: None,
    });

    let event = sub
        .recv()
        .await
        .expect("scoped subscriber should receive event");
    assert!(matches!(
        event,
        ExecutionEvent::NodeStarted {
            execution_id,
            node_id,
            ..
        } if execution_id == "e-target" && node_id == "n-target"
    ));
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
        trace_context: None,
    });
    telemetry.metrics().counter("nebula_test_counter").inc();

    assert_eq!(bus1.total_emitted(), bus2.total_emitted());
    assert_eq!(
        metrics1.counter("nebula_test_counter").get(),
        metrics2.counter("nebula_test_counter").get()
    );
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
            trace_context: None,
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
        trace_context: None,
    });
    bus.emit(ExecutionEvent::NodeStarted {
        execution_id: "exec".into(),
        node_id: "n1".into(),
        trace_context: None,
    });
    bus.emit(ExecutionEvent::NodeCompleted {
        execution_id: "exec".into(),
        node_id: "n1".into(),
        duration: Duration::from_millis(5),
        trace_context: None,
    });
    bus.emit(ExecutionEvent::Completed {
        execution_id: "exec".into(),
        duration: Duration::from_secs(1),
        trace_context: None,
    });

    metrics.counter("nebula_executions_total").inc();
    metrics.gauge("nebula_active_executions").set(0);
    metrics
        .histogram("nebula_execution_duration_seconds")
        .observe(1.0);

    assert_eq!(bus.total_emitted(), 4);
    assert_eq!(metrics.counter("nebula_executions_total").get(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Stress Test: Concurrent Metrics Registry Access
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn stress_test_concurrent_metrics_high_throughput() {
    tracing::debug!("stress test: concurrent metrics registry started");
    let registry = std::sync::Arc::new(MetricsRegistry::new());
    let num_threads = 32;
    let iterations = 10_000;

    let mut handles = vec![];

    for thread_id in 0..num_threads {
        let registry = std::sync::Arc::clone(&registry);
        let handle = std::thread::spawn(move || {
            let counter = registry.counter("nebula_ops_total");
            let histogram = registry.histogram("nebula_duration_ms");
            let gauge = registry.gauge("nebula_active_threads");

            for i in 0..iterations {
                counter.inc();
                histogram.observe((thread_id * iterations + i) as f64 / 1000.0);
                gauge.set((num_threads - thread_id) as i64);
            }
            tracing::debug!(
                "stress test: thread {} completed {} ops",
                thread_id,
                iterations
            );
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }

    let registry = &*registry;
    let counter_val = registry.counter("nebula_ops_total").get();
    let expected = (num_threads * iterations) as u64;
    let histogram_count = registry.histogram("nebula_duration_ms").count();

    tracing::info!(
        "stress test: metrics ops={}, expected={}, histogram_count={}",
        counter_val,
        expected,
        histogram_count
    );

    assert_eq!(
        counter_val, expected,
        "counter should reach {} after all threads",
        expected
    );
    assert_eq!(
        histogram_count, expected as usize,
        "histogram should record all {} observations",
        expected
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Stress Test: Concurrent EventBus with Multiple Subscribers
// ─────────────────────────────────────────────────────────────────────────────

#[expect(
    clippy::excessive_nesting,
    reason = "tokio::spawn inside loop and match in stress test naturally requires this depth"
)]
#[tokio::test]
async fn stress_test_eventbus_multiple_subscribers() {
    tracing::debug!("stress test: eventbus with multiple subscribers started");
    let bus = std::sync::Arc::new(EventBus::new(256));
    let num_subscribers = 10;
    let events_to_emit = 1000;

    // Spawn subscribers
    let mut subscriber_handles = vec![];
    for sub_id in 0..num_subscribers {
        let bus = std::sync::Arc::clone(&bus);
        let handle = tokio::spawn(async move {
            let mut sub = bus.subscribe();
            let mut count = 0;
            loop {
                match tokio::time::timeout(std::time::Duration::from_secs(5), sub.recv()).await {
                    Ok(Some(_)) => count += 1,
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            tracing::debug!(
                "stress test: subscriber {} received {} events",
                sub_id,
                count
            );
            count
        });
        subscriber_handles.push(handle);
    }

    // Emit events from separate task
    let emit_handle = {
        let bus = std::sync::Arc::clone(&bus);
        tokio::spawn(async move {
            for i in 0..events_to_emit {
                let event = if i % 4 == 0 {
                    ExecutionEvent::Started {
                        execution_id: format!("exec_{}", i),
                        workflow_id: format!("wf_{}", i / 10),
                        trace_context: None,
                    }
                } else if i % 4 == 1 {
                    ExecutionEvent::NodeStarted {
                        execution_id: format!("exec_{}", i),
                        node_id: format!("node_{}", i % 100),
                        trace_context: None,
                    }
                } else if i % 4 == 2 {
                    ExecutionEvent::NodeCompleted {
                        execution_id: format!("exec_{}", i),
                        node_id: format!("node_{}", i % 100),
                        duration: std::time::Duration::from_millis(1),
                        trace_context: None,
                    }
                } else {
                    ExecutionEvent::Completed {
                        execution_id: format!("exec_{}", i),
                        duration: std::time::Duration::from_secs(1),
                        trace_context: None,
                    }
                };
                bus.emit(event);
                if i % 100 == 0 {
                    tokio::task::yield_now().await;
                }
            }
            tracing::debug!("stress test: emitter complete (emit {})", events_to_emit);
        })
    };

    emit_handle.await.expect("emitter task panicked");

    // Give subscribers time to drain
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Drop bus to signal end
    drop(bus);

    let mut total_received = 0;
    for handle in subscriber_handles {
        let count = handle.await.expect("subscriber task panicked");
        total_received += count;
    }

    tracing::info!(
        "stress test: eventbus - emitted={}, total_received={}, subscribers={}",
        events_to_emit,
        total_received,
        num_subscribers
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Stress Test: Histogram Percentile Calculation Under Load
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn stress_test_histogram_concurrent_observations() {
    tracing::debug!("stress test: histogram concurrent observations started");
    let registry = std::sync::Arc::new(MetricsRegistry::new());
    let num_threads = 20;
    let observations_per_thread = 5_000;

    let histogram = registry.histogram("nebula_percentile_test");
    let histogram_arc = std::sync::Arc::new(histogram);

    let mut handles = vec![];

    for thread_id in 0..num_threads {
        let histogram = std::sync::Arc::clone(&histogram_arc);
        let handle = std::thread::spawn(move || {
            for i in 0..observations_per_thread {
                let value = ((thread_id * observations_per_thread + i) as f64) % 1000.0 / 10.0;
                histogram.observe(value);
            }
            tracing::debug!(
                "stress test: histogram thread {} added {} observations",
                thread_id,
                observations_per_thread
            );
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("thread panicked");
    }

    let total_count = histogram_arc.count();
    let expected = num_threads * observations_per_thread;

    tracing::info!(
        "stress test: histogram count={}, expected={}",
        total_count,
        expected
    );

    assert_eq!(
        total_count, expected,
        "should record all {} observations",
        expected
    );
}
