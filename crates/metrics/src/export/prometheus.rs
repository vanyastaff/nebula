//! Prometheus text format exporter.
//!
//! Renders metrics from a telemetry registry to Prometheus exposition format.
//! Use with an HTTP server to serve GET /metrics.
//!
//! The exporter iterates all entries in the registry (unlabeled **and**
//! labeled) via the `snapshot_*` APIs, groups them by metric name, and
//! renders each family with a single `# HELP` / `# TYPE` header followed by
//! sample lines. Labels are rendered as `{key1="value1",key2="value2"}`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use nebula_telemetry::labels::LabelInterner;
use nebula_telemetry::metrics::MetricsRegistry;

use crate::naming::{
    NEBULA_ACTION_DURATION_SECONDS, NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL,
    NEBULA_CACHE_EVICTIONS_TOTAL, NEBULA_CACHE_HITS_TOTAL, NEBULA_CACHE_MISSES_TOTAL,
    NEBULA_CACHE_SIZE, NEBULA_CREDENTIAL_ACTIVE_TOTAL, NEBULA_CREDENTIAL_EXPIRED_TOTAL,
    NEBULA_CREDENTIAL_ROTATIONS_TOTAL, NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
    NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL, NEBULA_EVENTBUS_DROP_RATIO_PPM,
    NEBULA_EVENTBUS_DROPPED, NEBULA_EVENTBUS_SENT, NEBULA_EVENTBUS_SUBSCRIBERS,
    NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_CLEANUP_TOTAL, NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
    NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS, NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};

/// Prometheus exposition format version (text-based).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Default Prometheus histogram bucket boundaries (in seconds).
const DEFAULT_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

// ── Static metric descriptors ─────────────────────────────────────────────────

fn counter_help(name: &str) -> &'static str {
    match name {
        NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL => "Total workflow executions started.",
        NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL => {
            "Total workflow executions completed successfully."
        }
        NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL => "Total workflow executions failed.",
        NEBULA_ACTION_EXECUTIONS_TOTAL => "Total action executions.",
        NEBULA_ACTION_FAILURES_TOTAL => "Total action failures.",
        NEBULA_RESOURCE_CREATE_TOTAL => "Total resource instances created.",
        NEBULA_RESOURCE_ACQUIRE_TOTAL => "Total resource acquisitions.",
        NEBULA_RESOURCE_RELEASE_TOTAL => "Total resource releases.",
        NEBULA_RESOURCE_CLEANUP_TOTAL => "Total resource cleanups.",
        NEBULA_RESOURCE_ERROR_TOTAL => "Total resource errors.",
        NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL => "Total pool exhaustion events.",
        NEBULA_RESOURCE_QUARANTINE_TOTAL => "Total resources quarantined.",
        NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL => "Total resources released from quarantine.",
        NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL => "Total config reloads.",
        NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL => "Total credential rotations applied.",
        NEBULA_CREDENTIAL_ROTATIONS_TOTAL => "Total credential rotation attempts.",
        NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL => "Total credential rotation failures.",
        NEBULA_CREDENTIAL_EXPIRED_TOTAL => "Total credentials expired.",
        NEBULA_CACHE_HITS_TOTAL => "Total cache hits.",
        NEBULA_CACHE_MISSES_TOTAL => "Total cache misses.",
        NEBULA_CACHE_EVICTIONS_TOTAL => "Total cache evictions.",
        _ => "Custom counter.",
    }
}

fn gauge_help(name: &str) -> &'static str {
    match name {
        NEBULA_RESOURCE_HEALTH_STATE => {
            "Resource health state (1=healthy, 0.5=degraded, 0=unhealthy)."
        }
        NEBULA_RESOURCE_POOL_WAITERS => "Number of waiters when pool exhausted.",
        NEBULA_EVENTBUS_SENT => "EventBus sent events snapshot.",
        NEBULA_EVENTBUS_DROPPED => "EventBus dropped events snapshot.",
        NEBULA_EVENTBUS_SUBSCRIBERS => "EventBus active subscribers snapshot.",
        NEBULA_EVENTBUS_DROP_RATIO_PPM => "EventBus drop ratio in parts-per-million.",
        NEBULA_CREDENTIAL_ACTIVE_TOTAL => "Number of active credentials.",
        NEBULA_CACHE_SIZE => "Current cache size in entries.",
        _ => "Custom gauge.",
    }
}

fn histogram_help(name: &str) -> &'static str {
    match name {
        NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS => "Workflow execution duration in seconds.",
        NEBULA_ACTION_DURATION_SECONDS => "Action execution duration in seconds.",
        NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS => {
            "Wait time before resource acquisition in seconds."
        }
        NEBULA_RESOURCE_USAGE_DURATION_SECONDS => "Resource usage duration in seconds.",
        NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS => {
            "Credential rotation duration in seconds."
        }
        _ => "Custom histogram.",
    }
}

// ── Label rendering ───────────────────────────────────────────────────────────

/// Render a Prometheus label selector string: `{k1="v1",k2="v2"}`.
///
/// Returns an empty string if the label set is empty (unlabeled metric).
fn render_labels(labels: &nebula_telemetry::labels::LabelSet, interner: &LabelInterner) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let mut out = String::from("{");
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let k_str = interner.resolve(k);
        let v_str = interner.resolve(v);
        // Escape double-quotes and backslashes in label values (Prometheus spec).
        let v_escaped = v_str.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = write!(out, "{}=\"{v_escaped}\"", k_str);
    }
    out.push('}');
    out
}

// ── snapshot function ─────────────────────────────────────────────────────────

/// Render the registry into Prometheus text exposition format.
///
/// Dynamically iterates all entries in the registry — including labeled
/// metrics — via `snapshot_counters`, `snapshot_gauges`, and
/// `snapshot_histograms`.  Entries are grouped by metric name so each family
/// gets a single `# HELP` / `# TYPE` header, matching the Prometheus
/// exposition format spec.
#[must_use]
pub fn snapshot(registry: &MetricsRegistry) -> String {
    let interner = registry.interner();
    let mut out = String::new();

    // ── Counters ──────────────────────────────────────────────────────────
    // Group by metric name so each family emits one HELP+TYPE header.
    let mut counter_families: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (key, counter) in registry.snapshot_counters() {
        let name = interner.resolve(key.name).to_owned();
        counter_families
            .entry(name)
            .or_default()
            .push((key.labels, counter));
    }
    for (name, entries) in &counter_families {
        let _ = writeln!(out, "# HELP {name} {}", counter_help(name));
        let _ = writeln!(out, "# TYPE {name} counter");
        for (labels, counter) in entries {
            let label_str = render_labels(labels, interner);
            let _ = writeln!(out, "{name}{label_str} {}", counter.get());
        }
    }

    // ── Gauges ────────────────────────────────────────────────────────────
    let mut gauge_families: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (key, gauge) in registry.snapshot_gauges() {
        let name = interner.resolve(key.name).to_owned();
        gauge_families
            .entry(name)
            .or_default()
            .push((key.labels, gauge));
    }
    for (name, entries) in &gauge_families {
        let _ = writeln!(out, "# HELP {name} {}", gauge_help(name));
        let _ = writeln!(out, "# TYPE {name} gauge");
        for (labels, gauge) in entries {
            let label_str = render_labels(labels, interner);
            let _ = writeln!(out, "{name}{label_str} {}", gauge.get());
        }
    }

    // ── Histograms ────────────────────────────────────────────────────────
    let mut histogram_families: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (key, histogram) in registry.snapshot_histograms() {
        let name = interner.resolve(key.name).to_owned();
        histogram_families
            .entry(name)
            .or_default()
            .push((key.labels, histogram));
    }
    for (name, entries) in &histogram_families {
        let _ = writeln!(out, "# HELP {name} {}", histogram_help(name));
        let _ = writeln!(out, "# TYPE {name} histogram");
        for (labels, hist) in entries {
            let count = hist.count();
            let sum = hist.sum();
            let buckets = hist.bucket_counts(DEFAULT_BUCKETS);
            let label_str = render_labels(labels, interner);
            // Insert le label into label set for bucket lines.
            for (le, cumulative) in &buckets {
                if label_str.is_empty() {
                    let _ = writeln!(out, "{name}_bucket{{le=\"{le}\"}} {cumulative}");
                } else {
                    // Merge existing labels with le — strip trailing `}` and append.
                    let merged = format!("{},le=\"{le}\"}}", &label_str[..label_str.len() - 1]);
                    let _ = writeln!(out, "{name}_bucket{merged} {cumulative}");
                }
            }
            // +Inf bucket
            if label_str.is_empty() {
                let _ = writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {count}");
                let _ = writeln!(out, "{name}_sum {sum}");
                let _ = writeln!(out, "{name}_count {count}");
            } else {
                let inf_labels = format!("{},le=\"+Inf\"}}", &label_str[..label_str.len() - 1]);
                let _ = writeln!(out, "{name}_bucket{inf_labels} {count}");
                let sum_labels = label_str.clone();
                let _ = writeln!(out, "{name}_sum{sum_labels} {sum}");
                let _ = writeln!(out, "{name}_count{sum_labels} {count}");
            }
        }
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
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"0.005\"} 1\n"),
            "bucket 0.005:\n{out}"
        );
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"0.025\"} 2\n"),
            "bucket 0.025:\n{out}"
        );
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"0.5\"} 3\n"),
            "bucket 0.5:\n{out}"
        );
        assert!(
            out.contains("nebula_action_duration_seconds_bucket{le=\"+Inf\"} 4\n"),
            "+Inf:\n{out}"
        );
    }

    #[test]
    fn empty_histogram_renders_all_zeros() {
        let registry = Arc::new(MetricsRegistry::new());
        let out = snapshot(&registry);
        // Empty registry — no histogram entries, nothing to render.
        // Recording one observation triggers rendering.
        registry
            .histogram("nebula_action_duration_seconds")
            .observe(0.0);
        let out2 = snapshot(&registry);
        assert!(out2.contains("nebula_action_duration_seconds_bucket{le=\"+Inf\"} 1\n"));
        // An empty registry should produce an empty string.
        assert!(out.is_empty(), "empty registry should produce no output");
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
    fn snapshot_renders_labeled_counters() {
        let registry = Arc::new(MetricsRegistry::new());
        let interner = registry.interner();
        let http_labels = interner.label_set(&[("action_type", "http.request")]);
        let math_labels = interner.label_set(&[("action_type", "math.add")]);

        registry
            .counter_labeled("nebula_action_executions_total", &http_labels)
            .inc_by(10);
        registry
            .counter_labeled("nebula_action_executions_total", &math_labels)
            .inc_by(3);

        let out = snapshot(&registry);
        assert!(
            out.contains("# TYPE nebula_action_executions_total counter"),
            "missing TYPE:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_action_executions_total{action_type="http.request"} 10"#),
            "missing http label:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_action_executions_total{action_type="math.add"} 3"#),
            "missing math label:\n{out}"
        );
    }

    #[test]
    fn snapshot_includes_help_and_type_lines() {
        let registry = Arc::new(MetricsRegistry::new());
        // Trigger creation of known metrics.
        registry
            .counter("nebula_workflow_executions_started_total")
            .inc();
        registry.gauge("nebula_resource_health_state").set(1);
        registry
            .histogram("nebula_workflow_execution_duration_seconds")
            .observe(1.0);

        let out = snapshot(&registry);
        assert!(out.contains("# HELP nebula_workflow_executions_started_total"));
        assert!(out.contains("# TYPE nebula_workflow_executions_started_total counter"));
        assert!(out.contains("# HELP nebula_resource_health_state"));
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
