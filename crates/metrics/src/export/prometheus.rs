//! Prometheus text format exporter.
//!
//! Renders metrics from a telemetry registry to Prometheus exposition format.
//! Use with an HTTP server to serve GET /metrics.

use std::sync::Arc;

use nebula_telemetry::metrics::MetricsRegistry;

use crate::naming::{
    NEBULA_ACTION_DURATION_SECONDS,
    NEBULA_ACTION_EXECUTIONS_TOTAL,
    NEBULA_ACTION_FAILURES_TOTAL,
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};

/// Prometheus exposition format version (text-based).
const PROMETHEUS_CONTENT_TYPE: &str =
    "text/plain; version=0.0.4; charset=utf-8";

/// Known counter names (workflow + action) for snapshot.
const COUNTER_NAMES: &[&str] = &[
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_ACTION_EXECUTIONS_TOTAL,
    NEBULA_ACTION_FAILURES_TOTAL,
];

/// Known histogram names for snapshot.
const HISTOGRAM_NAMES: &[&str] = &[
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
    NEBULA_ACTION_DURATION_SECONDS,
];

/// Render the registry into Prometheus text exposition format.
///
/// Includes only the known `nebula_*` counters and histograms.
/// Counters are output as `name total value`; histograms as
/// `name_bucket{le="+Inf"} count`, `name_sum`, `name_count`.
#[must_use]
pub fn snapshot(registry: &MetricsRegistry) -> String {
    let mut out = String::new();
    for name in COUNTER_NAMES {
        let value = registry.counter(name).get();
        out.push_str("# TYPE ");
        out.push_str(name);
        out.push_str(" counter\n");
        out.push_str(name);
        out.push_str(" ");
        out.push_str(&value.to_string());
        out.push('\n');
    }
    for name in HISTOGRAM_NAMES {
        let hist = registry.histogram(name);
        let count = hist.count();
        let sum = hist.sum();
        out.push_str("# TYPE ");
        out.push_str(name);
        out.push_str(" histogram\n");
        out.push_str(name);
        out.push_str("_bucket{le=\"+Inf\"} ");
        out.push_str(&count.to_string());
        out.push('\n');
        out.push_str(name);
        out.push_str("_sum ");
        out.push_str(&sum.to_string());
        out.push('\n');
        out.push_str(name);
        out.push_str("_count ");
        out.push_str(&count.to_string());
        out.push('\n');
    }
    out
}

/// Content-Type header value for Prometheus scrape.
#[must_use]
pub fn content_type() -> &'static str {
    PROMETHEUS_CONTENT_TYPE
}

/// Builder for a Prometheus metrics endpoint (e.g. for use with axum or hyper).
///
/// Holds a clone of the registry; call `snapshot()` on each request to render.
#[derive(Clone, Debug)]
pub struct PrometheusExporter {
    registry: Arc<MetricsRegistry>,
}

impl PrometheusExporter {
    /// Create an exporter that will snapshot the given registry.
    #[must_use]
    pub fn new(registry: Arc<MetricsRegistry>) -> Self {
        Self { registry }
    }

    /// Render current metrics in Prometheus text format.
    #[must_use]
    pub fn snapshot(&self) -> String {
        snapshot(&self.registry)
    }

    /// Content-Type for the response.
    #[must_use]
    pub fn content_type(&self) -> &'static str {
        content_type()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_telemetry::metrics::MetricsRegistry;

    use super::{snapshot, PrometheusExporter};

    #[test]
    fn snapshot_includes_counters_and_histograms() {
        let registry = Arc::new(MetricsRegistry::new());
        registry
            .counter("nebula_workflow_executions_started_total")
            .inc();
        registry
            .counter("nebula_workflow_executions_started_total")
            .inc();
        registry
            .histogram("nebula_action_duration_seconds")
            .observe(0.5);

        let out = snapshot(&registry);
        assert!(out.contains("nebula_workflow_executions_started_total"));
        assert!(out.contains(" 2\n"));
        assert!(out.contains("nebula_action_duration_seconds_bucket"));
        assert!(out.contains("nebula_action_duration_seconds_sum"));
        assert!(out.contains("nebula_action_duration_seconds_count"));
    }

    #[test]
    fn exporter_wraps_registry() {
        let registry = Arc::new(MetricsRegistry::new());
        registry
            .counter("nebula_action_failures_total")
            .inc_by(3);
        let exporter = PrometheusExporter::new(registry);
        let out = exporter.snapshot();
        assert!(out.contains("nebula_action_failures_total"));
        assert!(out.contains(" 3\n"));
    }
}
