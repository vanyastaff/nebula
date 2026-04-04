//! Telemetry service trait and implementations.
//!
//! [`TelemetryService`] is the main facade for the telemetry subsystem.
//! It provides access to the event bus and metrics registry.
//!
//! Two implementations are provided:
//! - [`NoopTelemetry`] — no-op (metrics in-memory, no export). Testing and MVP.
//! - [`ProductionTelemetry`] — configurable components with event bus and metrics.

use std::sync::Arc;

use crate::event::EventBus;
use crate::metrics::MetricsRegistry;

/// Telemetry service facade.
///
/// Provides access to the event bus and metrics registry.
/// Shared via `Arc<dyn TelemetryService>` across the engine and runtime.
///
/// Both [`EventBus`] and [`MetricsRegistry`] are cheaply cloneable (Arc-backed
/// internally), so callers that need an owned handle can simply `.clone()` the
/// reference returned here.
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

// ── ProductionTelemetry ──────────────────────────────────────────────────────

/// Production-ready telemetry service with configurable components.
///
/// Build via [`ProductionTelemetryBuilder`].
///
/// # Examples
///
/// ```no_run
/// use nebula_telemetry::service::{ProductionTelemetry, TelemetryService};
///
/// # async fn example() {
/// let telemetry = ProductionTelemetry::builder().build();
/// let counter = telemetry.metrics().counter("nebula_executions_total");
/// counter.inc();
/// # }
/// ```
pub struct ProductionTelemetry {
    event_bus: EventBus,
    metrics: MetricsRegistry,
}

impl ProductionTelemetry {
    /// Create a builder for configuring a production telemetry service.
    #[must_use]
    pub fn builder() -> ProductionTelemetryBuilder {
        ProductionTelemetryBuilder::default()
    }

    /// Create as an `Arc<dyn TelemetryService>` for dependency injection.
    pub fn arc(self) -> Arc<dyn TelemetryService> {
        Arc::new(self)
    }
}

impl TelemetryService for ProductionTelemetry {
    fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }

    fn metrics(&self) -> &MetricsRegistry {
        &self.metrics
    }
}

// ── ProductionTelemetryBuilder ───────────────────────────────────────────────

/// Builder for [`ProductionTelemetry`].
///
/// All components have sensible defaults:
/// - Event bus: capacity 1024
/// - Metrics: new in-memory registry
#[derive(Default)]
pub struct ProductionTelemetryBuilder {
    event_bus: Option<EventBus>,
    metrics: Option<MetricsRegistry>,
}

impl ProductionTelemetryBuilder {
    /// Set a custom event bus.
    #[must_use]
    pub fn with_event_bus(mut self, event_bus: EventBus) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Set a custom metrics registry.
    #[must_use]
    pub fn with_metrics(mut self, metrics: MetricsRegistry) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Build the production telemetry service.
    #[must_use]
    pub fn build(self) -> ProductionTelemetry {
        let event_bus = self.event_bus.unwrap_or_else(|| EventBus::new(1024));
        let metrics = self.metrics.unwrap_or_default();

        tracing::info!("production telemetry service initialized");

        ProductionTelemetry { event_bus, metrics }
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
            trace_context: None,
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

    #[test]
    fn production_telemetry_builder_defaults() {
        let telemetry = ProductionTelemetry::builder().build();
        telemetry.metrics().counter("test").inc();
        assert_eq!(telemetry.metrics().counter("test").get(), 1);
    }

    #[test]
    fn production_telemetry_custom_components() {
        let bus = EventBus::new(64);
        let metrics = MetricsRegistry::new();

        let telemetry = ProductionTelemetry::builder()
            .with_event_bus(bus.clone())
            .with_metrics(metrics.clone())
            .build();

        telemetry.metrics().counter("shared").inc();
        assert_eq!(metrics.counter("shared").get(), 1);
    }

    #[test]
    fn production_telemetry_arc_is_object_safe() {
        let t: Arc<dyn TelemetryService> = ProductionTelemetry::builder().build().arc();
        t.metrics().counter("x").inc();
        assert_eq!(t.metrics().counter("x").get(), 1);
    }
}
