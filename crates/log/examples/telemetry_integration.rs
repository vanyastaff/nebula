//! End-to-end example: wiring `nebula-log` with a custom telemetry sink.
//!
//! This example shows the integration pattern you would use to connect
//! `nebula-log`'s observability hook system to any telemetry backend (e.g.
//! `nebula-telemetry`, Prometheus, a custom metrics sink).
//!
//! The `RecordingHook` below acts as a stand-in for a real telemetry backend —
//! it records events in memory so the example can assert they were delivered
//! without requiring a running collector.
//!
//! # Running
//!
//! ```text
//! cargo run --example telemetry_integration -p nebula-log
//! ```

use nebula_log::{
    Config, Format, WriterConfig,
    observability::{
        ObservabilityEvent, ObservabilityHook, OperationCompleted, OperationFailed,
        OperationStarted, OperationTracker, emit_event, register_hook,
    },
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── Step 1: Define a telemetry-forwarding hook ───────────────────────────────

/// A hook that captures event names for offline assertion.
///
/// In production this would forward structured fields to an OpenTelemetry
/// exporter, a Prometheus counter, or any other telemetry backend.
struct RecordingHook {
    events: Arc<Mutex<Vec<String>>>,
}

impl ObservabilityHook for RecordingHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        tracing::debug!(event_name = event.name(), "RecordingHook received event");
        self.events.lock().unwrap().push(event.name().to_string());
    }

    fn initialize(&self) {
        tracing::debug!("RecordingHook initialised — ready to forward events");
    }

    fn shutdown(&self) {
        tracing::debug!("RecordingHook shutting down");
    }
}

// ── Step 2: Set up logging ───────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a compact dev config so the example output is readable.
    let mut cfg = Config::development();
    cfg.format = Format::Compact;
    cfg.writer = WriterConfig::Stderr;
    cfg.fields.service = Some("telemetry-example".to_string());

    // Keep the guard alive for the process lifetime.
    let _guard = nebula_log::init_with(cfg)?;

    tracing::info!("logging initialised");

    // ── Step 3: Register the telemetry-forwarding hook ───────────────────────

    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    register_hook(Arc::new(RecordingHook {
        events: Arc::clone(&captured),
    }));

    tracing::debug!("RecordingHook registered");

    // ── Step 4: Emit operation lifecycle events ───────────────────────────────
    //
    // Any call to `emit_event` dispatches to every registered hook.
    // In real services you would emit these at operation boundaries (HTTP
    // handler entry/exit, database calls, workflow node execution, etc.).

    emit_event(&OperationStarted {
        operation: "workflow.execute".to_string(),
        context: "run-id=abc123".to_string(),
    });

    // Simulate work.
    tracing::info!(workflow_id = "abc123", "executing workflow nodes");
    std::thread::sleep(Duration::from_millis(5));

    emit_event(&OperationCompleted {
        operation: "workflow.execute".to_string(),
        duration: Duration::from_millis(5),
    });

    // ── Step 5: Use OperationTracker for automatic start/complete pairs ───────
    //
    // `OperationTracker::new("name", "context")` emits `OperationStarted`
    // immediately. Calling `.success()` emits `OperationCompleted`; calling
    // `.fail(msg)` or just dropping the tracker emits `OperationFailed`.

    let tracker = OperationTracker::new("credential.rotate", "scheduled-rotation");
    tracing::debug!("running credential rotation");
    tracker.success();

    // Simulate a failure path.
    emit_event(&OperationFailed {
        operation: "workflow.schedule".to_string(),
        error: "queue is full".to_string(),
        duration: Duration::from_millis(1),
    });

    // ── Step 6: Assert delivery (in tests; omit in production) ───────────────

    let received = captured.lock().unwrap().clone();
    tracing::info!(events = ?received, "all events captured by telemetry hook");

    assert!(
        received.contains(&"operation_started".to_string()),
        "missing start"
    );
    assert!(
        received.contains(&"operation_completed".to_string()),
        "missing complete"
    );
    assert!(
        received.contains(&"operation_failed".to_string()),
        "missing failure"
    );

    tracing::info!(
        "example complete — {} events forwarded to RecordingHook",
        received.len()
    );

    // Notes on extending this pattern:
    //
    //   • Replace `RecordingHook` with a real telemetry backend hook from
    //     `nebula-telemetry` once it stabilises (Phase 3+).
    //   • For OTLP traces, enable `features = ["telemetry"]` and set
    //     `OTEL_EXPORTER_OTLP_ENDPOINT`. See `examples/otlp_setup.rs`.
    //   • For Sentry error tracking, enable `features = ["sentry"]` and set
    //     `SENTRY_DSN`. See `examples/sentry_setup.rs`.

    Ok(())
}
