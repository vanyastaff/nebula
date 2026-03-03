//! Telemetry service trait and implementations.
//!
//! [`TelemetryService`] is the main facade for the telemetry subsystem.
//! It provides access to the event bus, metrics registry, and execution recorder.

use std::sync::Arc;

use crate::event::EventBus;
use crate::metrics::MetricsRegistry;
use crate::trace::Recorder;

/// Telemetry service facade.
///
/// Provides access to the event bus, metrics registry, and execution recorder.
/// Shared via `Arc<dyn TelemetryService>` across the engine and runtime.
pub trait TelemetryService: Send + Sync {
    /// Access the event bus for emitting and subscribing to events.
    fn event_bus(&self) -> &EventBus;

    /// Access the metrics registry for recording metrics.
    fn metrics(&self) -> &MetricsRegistry;

    /// Return a shared handle to the event bus for wiring into engine/runtime.
    fn event_bus_arc(&self) -> Arc<EventBus>;

    /// Return a shared handle to the metrics registry for wiring into engine/runtime.
    fn metrics_arc(&self) -> Arc<MetricsRegistry>;

    /// Return the execution recorder for resource usage and call traces.
    /// Engine injects this into resource context so one backend receives all trace data.
    fn execution_recorder(&self) -> Arc<dyn Recorder>;
}

/// No-op telemetry implementation.
///
/// Events are silently dropped (no subscribers). Metrics are recorded
/// in memory but never exported. Suitable for testing, development,
/// and the desktop MVP.
///
/// # Examples
///
/// ```
/// use nebula_telemetry::service::{NoopTelemetry, TelemetryService};
///
/// let telemetry = NoopTelemetry::new();
/// let counter = telemetry.metrics().counter("test");
/// counter.inc();
/// assert_eq!(counter.get(), 1);
/// ```
pub struct NoopTelemetry {
    event_bus: Arc<EventBus>,
    metrics: Arc<MetricsRegistry>,
    recorder: Arc<dyn Recorder>,
}

impl NoopTelemetry {
    /// Create a new no-op telemetry service.
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_bus: Arc::new(EventBus::new(128)),
            metrics: Arc::new(MetricsRegistry::new()),
            recorder: Arc::new(crate::trace::NoopRecorder),
        }
    }

    /// Create as an `Arc<dyn TelemetryService>` for dependency injection.
    #[must_use]
    pub fn arc() -> Arc<dyn TelemetryService> {
        Arc::new(Self::new())
    }
}

impl Default for NoopTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl TelemetryService for NoopTelemetry {
    fn event_bus(&self) -> &EventBus {
        self.event_bus.as_ref()
    }

    fn metrics(&self) -> &MetricsRegistry {
        self.metrics.as_ref()
    }

    fn event_bus_arc(&self) -> Arc<EventBus> {
        Arc::clone(&self.event_bus)
    }

    fn metrics_arc(&self) -> Arc<MetricsRegistry> {
        Arc::clone(&self.metrics)
    }

    fn execution_recorder(&self) -> Arc<dyn Recorder> {
        Arc::clone(&self.recorder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ExecutionEvent;

    #[test]
    fn noop_telemetry_does_not_panic() {
        let telemetry = NoopTelemetry::new();
        telemetry.event_bus().emit(ExecutionEvent::Started {
            execution_id: "e1".into(),
            workflow_id: "w1".into(),
        });
        telemetry.metrics().counter("test").inc();
        telemetry.metrics().gauge("active").set(5);
        telemetry.metrics().histogram("duration").observe(1.23);
    }

    #[test]
    fn noop_telemetry_arc_is_object_safe() {
        let t: Arc<dyn TelemetryService> = NoopTelemetry::arc();
        t.metrics().counter("x").inc();
    }
}
