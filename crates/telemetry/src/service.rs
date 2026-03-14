//! Telemetry service trait and implementations.
//!
//! [`TelemetryService`] is the main facade for the telemetry subsystem.
//! It provides access to the event bus, metrics registry, and execution recorder.
//!
//! Two implementations are provided:
//! - [`NoopTelemetry`] — no-op (metrics in-memory, no export). Testing and MVP.
//! - [`ProductionTelemetry`] — configurable components with buffered recording.

use std::sync::Arc;

use crate::event::EventBus;
use crate::metrics::MetricsRegistry;
use crate::recorder::{BufferedRecorder, BufferedRecorderConfig, LogSink};
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

// ── ProductionTelemetry ──────────────────────────────────────────────────────

/// Production-ready telemetry service with configurable components.
///
/// Uses [`BufferedRecorder`] by default for non-blocking record collection.
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
    event_bus: Arc<EventBus>,
    metrics: Arc<MetricsRegistry>,
    recorder: Arc<dyn Recorder>,
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

// ── ProductionTelemetryBuilder ───────────────────────────────────────────────

/// Builder for [`ProductionTelemetry`].
///
/// All components have sensible defaults:
/// - Event bus: capacity 1024
/// - Metrics: new in-memory registry
/// - Recorder: [`BufferedRecorder`] with [`LogSink`] and default config
#[derive(Default)]
pub struct ProductionTelemetryBuilder {
    event_bus: Option<Arc<EventBus>>,
    metrics: Option<Arc<MetricsRegistry>>,
    recorder: Option<Arc<dyn Recorder>>,
    buffer_config: Option<BufferedRecorderConfig>,
}



impl ProductionTelemetryBuilder {
    /// Set a custom event bus.
    #[must_use]
    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Set a custom metrics registry.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<MetricsRegistry>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Set a custom recorder (overrides buffer config).
    #[must_use]
    pub fn with_recorder(mut self, recorder: Arc<dyn Recorder>) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Configure the default [`BufferedRecorder`].
    ///
    /// Ignored if a custom recorder is set via [`with_recorder`](Self::with_recorder).
    #[must_use]
    pub fn with_buffer_config(mut self, config: BufferedRecorderConfig) -> Self {
        self.buffer_config = Some(config);
        self
    }

    /// Build the production telemetry service.
    ///
    /// Must be called within a tokio runtime (spawns background tasks).
    #[must_use]
    pub fn build(self) -> ProductionTelemetry {
        let event_bus = self
            .event_bus
            .unwrap_or_else(|| Arc::new(EventBus::new(1024)));
        let metrics = self
            .metrics
            .unwrap_or_else(|| Arc::new(MetricsRegistry::new()));
        let recorder = self.recorder.unwrap_or_else(|| {
            let config = self.buffer_config.unwrap_or_default();
            Arc::new(BufferedRecorder::start(config, LogSink))
        });

        tracing::info!("production telemetry service initialized");

        ProductionTelemetry {
            event_bus,
            metrics,
            recorder,
        }
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

    #[tokio::test]
    async fn production_telemetry_builder_defaults() {
        let telemetry = ProductionTelemetry::builder().build();
        telemetry.metrics().counter("test").inc();
        assert_eq!(telemetry.metrics().counter("test").get(), 1);
    }

    #[tokio::test]
    async fn production_telemetry_custom_components() {
        let bus = Arc::new(EventBus::new(64));
        let metrics = Arc::new(MetricsRegistry::new());

        let telemetry = ProductionTelemetry::builder()
            .with_event_bus(Arc::clone(&bus))
            .with_metrics(Arc::clone(&metrics))
            .build();

        telemetry.metrics().counter("shared").inc();
        assert_eq!(metrics.counter("shared").get(), 1);
    }

    #[tokio::test]
    async fn production_telemetry_arc_is_object_safe() {
        let t: Arc<dyn TelemetryService> = ProductionTelemetry::builder().build().arc();
        t.metrics().counter("x").inc();
        assert_eq!(t.metrics().counter("x").get(), 1);
    }

    #[tokio::test]
    async fn production_telemetry_execution_recorder_works() {
        let telemetry = ProductionTelemetry::builder().build();
        let recorder = telemetry.execution_recorder();
        // Should not panic — recorder accepts records
        assert!(recorder.is_enrichment_enabled());
    }
}
