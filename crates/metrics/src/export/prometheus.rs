//! Prometheus text format exporter.
//!
//! Renders metrics from a telemetry registry to Prometheus exposition format.
//! Use with an HTTP server to serve GET /metrics.

use std::fmt::Write as _;
use std::sync::Arc;

use nebula_telemetry::metrics::MetricsRegistry;

use crate::naming::{
    NEBULA_ACTION_DURATION_SECONDS, NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL,
    NEBULA_EVENTBUS_DROP_RATIO_PPM, NEBULA_EVENTBUS_DROPPED, NEBULA_EVENTBUS_SENT,
    NEBULA_EVENTBUS_SUBSCRIBERS, NEBULA_RESOURCE_ACQUIRE_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS, NEBULA_RESOURCE_CLEANUP_TOTAL,
    NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL, NEBULA_RESOURCE_CREATE_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL, NEBULA_RESOURCE_ERROR_TOTAL,
    NEBULA_RESOURCE_HEALTH_STATE, NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL,
    NEBULA_RESOURCE_POOL_WAITERS, NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL,
    NEBULA_RESOURCE_QUARANTINE_TOTAL, NEBULA_RESOURCE_RELEASE_TOTAL,
    NEBULA_RESOURCE_USAGE_DURATION_SECONDS, NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};

/// Prometheus exposition format version (text-based).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Default Prometheus histogram bucket boundaries (in seconds).
const DEFAULT_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Counter metric descriptor.
struct CounterDesc {
    name: &'static str,
    help: &'static str,
}

/// Gauge metric descriptor.
struct GaugeDesc {
    name: &'static str,
    help: &'static str,
}

/// Histogram metric descriptor.
struct HistogramDesc {
    name: &'static str,
    help: &'static str,
}

/// All known counter metrics.
const COUNTERS: &[CounterDesc] = &[
    CounterDesc {
        name: NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
        help: "Total workflow executions started.",
    },
    CounterDesc {
        name: NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
        help: "Total workflow executions completed successfully.",
    },
    CounterDesc {
        name: NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
        help: "Total workflow executions failed.",
    },
    CounterDesc {
        name: NEBULA_ACTION_EXECUTIONS_TOTAL,
        help: "Total action executions.",
    },
    CounterDesc {
        name: NEBULA_ACTION_FAILURES_TOTAL,
        help: "Total action failures.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_CREATE_TOTAL,
        help: "Total resource instances created.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_ACQUIRE_TOTAL,
        help: "Total resource acquisitions.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_RELEASE_TOTAL,
        help: "Total resource releases.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_CLEANUP_TOTAL,
        help: "Total resource cleanups.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_ERROR_TOTAL,
        help: "Total resource errors.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL,
        help: "Total pool exhaustion events.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_QUARANTINE_TOTAL,
        help: "Total resources quarantined.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL,
        help: "Total resources released from quarantine.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
        help: "Total config reloads.",
    },
    CounterDesc {
        name: NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
        help: "Total credential rotations applied.",
    },
];

/// All known gauge metrics.
const GAUGES: &[GaugeDesc] = &[
    GaugeDesc {
        name: NEBULA_RESOURCE_HEALTH_STATE,
        help: "Resource health state (1=healthy, 0.5=degraded, 0=unhealthy).",
    },
    GaugeDesc {
        name: NEBULA_RESOURCE_POOL_WAITERS,
        help: "Number of waiters when pool exhausted.",
    },
    GaugeDesc {
        name: NEBULA_EVENTBUS_SENT,
        help: "EventBus sent events snapshot.",
    },
    GaugeDesc {
        name: NEBULA_EVENTBUS_DROPPED,
        help: "EventBus dropped events snapshot.",
    },
    GaugeDesc {
        name: NEBULA_EVENTBUS_SUBSCRIBERS,
        help: "EventBus active subscribers snapshot.",
    },
    GaugeDesc {
        name: NEBULA_EVENTBUS_DROP_RATIO_PPM,
        help: "EventBus drop ratio in parts-per-million.",
    },
];

/// All known histogram metrics.
const HISTOGRAMS: &[HistogramDesc] = &[
    HistogramDesc {
        name: NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
        help: "Workflow execution duration in seconds.",
    },
    HistogramDesc {
        name: NEBULA_ACTION_DURATION_SECONDS,
        help: "Action execution duration in seconds.",
    },
    HistogramDesc {
        name: NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
        help: "Wait time before resource acquisition in seconds.",
    },
    HistogramDesc {
        name: NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
        help: "Resource usage duration in seconds.",
    },
];

/// Render the registry into Prometheus text exposition format.
///
/// Includes all known `nebula_*` counters, gauges, and histograms with
/// `# HELP` and `# TYPE` metadata lines. Histograms include per-bucket
/// cumulative counts using default Prometheus boundaries.
#[must_use]
pub fn snapshot(registry: &MetricsRegistry) -> String {
    let mut out = String::new();

    for desc in COUNTERS {
        let value = registry.counter(desc.name).get();
        let _ = writeln!(out, "# HELP {} {}", desc.name, desc.help);
        let _ = writeln!(out, "# TYPE {} counter", desc.name);
        let _ = writeln!(out, "{} {value}", desc.name);
    }

    for desc in GAUGES {
        let value = registry.gauge(desc.name).get();
        let _ = writeln!(out, "# HELP {} {}", desc.name, desc.help);
        let _ = writeln!(out, "# TYPE {} gauge", desc.name);
        let _ = writeln!(out, "{} {value}", desc.name);
    }

    for desc in HISTOGRAMS {
        let hist = registry.histogram(desc.name);
        let count = hist.count();
        let sum = hist.sum();
        let buckets = hist.bucket_counts(DEFAULT_BUCKETS);

        let _ = writeln!(out, "# HELP {} {}", desc.name, desc.help);
        let _ = writeln!(out, "# TYPE {} histogram", desc.name);
        for (le, cumulative) in &buckets {
            let _ = writeln!(out, "{}_bucket{{le=\"{le}\"}} {cumulative}", desc.name);
        }
        let _ = writeln!(out, "{}_bucket{{le=\"+Inf\"}} {count}", desc.name);
        let _ = writeln!(out, "{}_sum {sum}", desc.name);
        let _ = writeln!(out, "{}_count {count}", desc.name);
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

    use super::{PrometheusExporter, snapshot};

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
        assert!(out.contains("# HELP nebula_workflow_executions_started_total"));
        assert!(out.contains("# TYPE nebula_workflow_executions_started_total counter"));
        assert!(out.contains("nebula_workflow_executions_started_total 2\n"));
        assert!(out.contains("nebula_action_duration_seconds_bucket{le=\"+Inf\"} 1\n"));
        assert!(out.contains("nebula_action_duration_seconds_sum 0.5\n"));
        assert!(out.contains("nebula_action_duration_seconds_count 1\n"));
    }

    #[test]
    fn histogram_renders_per_bucket_counts() {
        let registry = Arc::new(MetricsRegistry::new());
        let hist = registry.histogram("nebula_action_duration_seconds");
        hist.observe(0.003); // <= 0.005
        hist.observe(0.02); // <= 0.025
        hist.observe(0.5); // <= 0.5
        hist.observe(3.0); // <= 5.0

        let out = snapshot(&registry);
        // 0.005 bucket: 1 observation (0.003)
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"0.005\"} 1\n"),
            "bucket 0.005:\n{out}"
        );
        // 0.025 bucket: 2 observations (0.003, 0.02)
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"0.025\"} 2\n"),
            "bucket 0.025:\n{out}"
        );
        // 0.5 bucket: 3 observations
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"0.5\"} 3\n"),
            "bucket 0.5:\n{out}"
        );
        // 5.0 bucket: 4 observations
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"5\"} 4\n"),
            "bucket 5.0:\n{out}"
        );
        // +Inf: 4 total
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"+Inf\"} 4\n"),
            "+Inf:\n{out}"
        );
    }

    #[test]
    fn empty_histogram_renders_all_zeros() {
        let registry = Arc::new(MetricsRegistry::new());
        let out = snapshot(&registry);
        assert!(out.contains("nebula_action_duration_seconds_bucket{le=\"+Inf\"} 0\n"));
        assert!(out.contains("nebula_action_duration_seconds_sum 0\n"));
        assert!(out.contains("nebula_action_duration_seconds_count 0\n"));
    }

    #[test]
    fn snapshot_includes_resource_metrics() {
        let registry = Arc::new(MetricsRegistry::new());
        registry.counter("nebula_resource_create_total").inc_by(5);
        registry.counter("nebula_resource_error_total").inc();

        let out = snapshot(&registry);
        assert!(out.contains("# TYPE nebula_resource_create_total counter"));
        assert!(out.contains("nebula_resource_create_total 5\n"));
        assert!(out.contains("nebula_resource_error_total 1\n"));
    }

    #[test]
    fn snapshot_includes_eventbus_metrics() {
        let registry = Arc::new(MetricsRegistry::new());
        registry.gauge("nebula_eventbus_sent").set(100);
        registry.gauge("nebula_eventbus_dropped").set(5);
        registry.gauge("nebula_eventbus_subscribers").set(3);

        let out = snapshot(&registry);
        assert!(out.contains("# TYPE nebula_eventbus_sent gauge"));
        assert!(out.contains("nebula_eventbus_sent 100\n"));
        assert!(out.contains("nebula_eventbus_dropped 5\n"));
        assert!(out.contains("nebula_eventbus_subscribers 3\n"));
    }

    #[test]
    fn snapshot_includes_help_and_type_lines() {
        let registry = Arc::new(MetricsRegistry::new());
        let out = snapshot(&registry);

        // Every metric should have HELP and TYPE
        assert!(out.contains("# HELP nebula_workflow_executions_started_total"));
        assert!(out.contains("# TYPE nebula_workflow_executions_started_total counter"));
        assert!(out.contains("# HELP nebula_resource_health_state"));
        assert!(out.contains("# TYPE nebula_resource_health_state gauge"));
        assert!(out.contains("# HELP nebula_workflow_execution_duration_seconds"));
        assert!(out.contains("# TYPE nebula_workflow_execution_duration_seconds histogram"));
    }

    #[test]
    fn exporter_wraps_registry() {
        let registry = Arc::new(MetricsRegistry::new());
        registry.counter("nebula_action_failures_total").inc_by(3);
        let exporter = PrometheusExporter::new(registry);
        let out = exporter.snapshot();
        assert!(out.contains("nebula_action_failures_total 3\n"));
    }
}
