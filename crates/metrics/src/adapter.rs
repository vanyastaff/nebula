//! Adapter over `nebula-telemetry::MetricsRegistry` with standard `nebula_*` names.
//!
//! Use this when you want to record metrics with the unified naming convention
//! without changing the underlying telemetry registry.

use std::sync::Arc;

use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};

use crate::naming::*;

/// Adapter that exposes telemetry metrics under standard `nebula_*` names.
///
/// Holds an `Arc<MetricsRegistry>` and provides accessors that return
/// counters/gauges/histograms with the canonical names. Engine and runtime
/// can use this to record metrics that are ready for Prometheus/OTLP export.
#[derive(Clone, Debug)]
pub struct TelemetryAdapter {
    registry: Arc<MetricsRegistry>,
}

impl TelemetryAdapter {
    /// Create an adapter over the given telemetry registry.
    #[must_use]
    pub fn new(registry: Arc<MetricsRegistry>) -> Self {
        Self { registry }
    }

    /// Underlying registry for custom or legacy metric names.
    #[must_use]
    pub fn registry(&self) -> &MetricsRegistry {
        &self.registry
    }

    // ---------- Workflow (engine) ----------

    /// Counter: workflow executions started.
    #[must_use]
    pub fn workflow_executions_started_total(&self) -> Counter {
        self.registry
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
    }

    /// Counter: workflow executions completed successfully.
    #[must_use]
    pub fn workflow_executions_completed_total(&self) -> Counter {
        self.registry
            .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
    }

    /// Counter: workflow executions failed.
    #[must_use]
    pub fn workflow_executions_failed_total(&self) -> Counter {
        self.registry
            .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
    }

    /// Histogram: workflow execution duration in seconds.
    #[must_use]
    pub fn workflow_execution_duration_seconds(&self) -> Histogram {
        self.registry
            .histogram(NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS)
    }

    // ---------- Action (runtime) ----------

    /// Counter: action executions (success + failure).
    #[must_use]
    pub fn action_executions_total(&self) -> Counter {
        self.registry.counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
    }

    /// Counter: action failures.
    #[must_use]
    pub fn action_failures_total(&self) -> Counter {
        self.registry.counter(NEBULA_ACTION_FAILURES_TOTAL)
    }

    /// Histogram: action execution duration in seconds.
    #[must_use]
    pub fn action_duration_seconds(&self) -> Histogram {
        self.registry.histogram(NEBULA_ACTION_DURATION_SECONDS)
    }

    // ---------- Generic access (for domain-specific names) ----------

    /// Get or create a counter by name. Prefer the typed accessors above when available.
    #[must_use]
    pub fn counter(&self, name: &str) -> Counter {
        self.registry.counter(name)
    }

    /// Get or create a gauge by name.
    #[must_use]
    pub fn gauge(&self, name: &str) -> Gauge {
        self.registry.gauge(name)
    }

    /// Get or create a histogram by name.
    #[must_use]
    pub fn histogram(&self, name: &str) -> Histogram {
        self.registry.histogram(name)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_telemetry::metrics::MetricsRegistry;

    use super::TelemetryAdapter;

    #[test]
    fn adapter_records_under_standard_names() {
        let registry = Arc::new(MetricsRegistry::new());
        let adapter = TelemetryAdapter::new(Arc::clone(&registry));

        adapter.workflow_executions_started_total().inc();
        adapter.workflow_executions_started_total().inc();
        adapter.action_executions_total().inc();
        adapter.action_failures_total().inc();

        assert_eq!(adapter.workflow_executions_started_total().get(), 2);
        assert_eq!(adapter.action_executions_total().get(), 1);
        assert_eq!(adapter.action_failures_total().get(), 1);
    }
}
