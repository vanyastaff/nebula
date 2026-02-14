//! Telemetry service trait and implementations.
//!
//! [`TelemetryService`] is the main facade for the telemetry subsystem.
//! It provides access to the event bus and metrics registry.

use std::sync::Arc;

use crate::event::EventBus;
use crate::metrics::MetricsRegistry;

/// Telemetry service facade.
///
/// Provides access to the event bus and metrics registry. Shared via
/// `Arc<dyn TelemetryService>` across the engine and runtime.
pub trait TelemetryService: Send + Sync {
    /// Access the event bus for emitting and subscribing to events.
    fn event_bus(&self) -> &EventBus;

    /// Access the metrics registry for recording metrics.
    fn metrics(&self) -> &MetricsRegistry;
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
    event_bus: EventBus,
    metrics: MetricsRegistry,
}

impl NoopTelemetry {
    /// Create a new no-op telemetry service.
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_bus: EventBus::new(128),
            metrics: MetricsRegistry::new(),
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
        &self.event_bus
    }

    fn metrics(&self) -> &MetricsRegistry {
        &self.metrics
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
