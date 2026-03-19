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

use crate::naming::{ALL_METRICS, MetricName};

/// Prometheus exposition format version (text-based).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Default Prometheus histogram bucket boundaries (in seconds).
const DEFAULT_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

// ── Static metric descriptors ─────────────────────────────────────────────────

/// Look up a well-known metric by its Prometheus name string.
fn lookup_metric(name: &str) -> Option<MetricName> {
    ALL_METRICS.iter().find(|m| m.as_str() == name).copied()
}

/// Return the HELP text for a metric, falling back to a generic message.
fn metric_help(name: &str) -> &'static str {
    lookup_metric(name).map_or("Custom metric.", |m| m.help())
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
        let _ = writeln!(out, "# HELP {name} {}", metric_help(name));
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
        let _ = writeln!(out, "# HELP {name} {}", metric_help(name));
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
        let _ = writeln!(out, "# HELP {name} {}", metric_help(name));
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
    fn snapshot_escapes_quotes_in_label_values() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry
            .interner()
            .label_set(&[("path", "/api?foo=\"bar\"")]);
        registry
            .counter_labeled("nebula_action_executions_total", &labels)
            .inc();

        let out = snapshot(&registry);
        assert!(
            out.contains(r#"nebula_action_executions_total{path="/api?foo=\"bar\""} 1"#),
            "escaped quotes:\n{out}"
        );
    }

    #[test]
    fn snapshot_escapes_backslashes_in_label_values() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry
            .interner()
            .label_set(&[("path", "C:\\Users\\test")]);
        registry
            .counter_labeled("nebula_action_executions_total", &labels)
            .inc();

        let out = snapshot(&registry);
        assert!(
            out.contains(r#"nebula_action_executions_total{path="C:\\Users\\test"} 1"#),
            "escaped backslashes:\n{out}"
        );
    }

    #[test]
    fn snapshot_escapes_quotes_and_backslashes_together() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry
            .interner()
            .label_set(&[("val", r#"say \"hello\""#)]);
        registry
            .counter_labeled("nebula_action_executions_total", &labels)
            .inc();

        let out = snapshot(&registry);
        assert!(
            out.contains(r#"nebula_action_executions_total{val="say \\\"hello\\\""} 1"#),
            "mixed escaping:\n{out}"
        );
    }

    #[test]
    fn snapshot_renders_labeled_histogram_with_bucket_labels() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry
            .interner()
            .label_set(&[("action_type", "http.request")]);
        registry
            .histogram_labeled("nebula_action_duration_seconds", &labels)
            .observe(0.5);

        let out = snapshot(&registry);
        assert!(
            out.contains(
                r#"nebula_action_duration_seconds_bucket{action_type="http.request",le="0.5"}"#
            ),
            "labeled bucket:\n{out}"
        );
        assert!(
            out.contains(
                r#"nebula_action_duration_seconds_bucket{action_type="http.request",le="+Inf"}"#
            ),
            "labeled +Inf:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_action_duration_seconds_sum{action_type="http.request"}"#),
            "labeled sum:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_action_duration_seconds_count{action_type="http.request"}"#),
            "labeled count:\n{out}"
        );
    }

    #[test]
    fn snapshot_renders_labeled_gauge() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry
            .interner()
            .label_set(&[("resource_type", "database")]);
        registry
            .gauge_labeled("nebula_resource_health_state", &labels)
            .set(1);

        let out = snapshot(&registry);
        assert!(
            out.contains(r#"nebula_resource_health_state{resource_type="database"} 1"#),
            "labeled gauge:\n{out}"
        );
    }

    #[test]
    fn content_type_returns_prometheus_exposition_format() {
        assert_eq!(
            super::content_type(),
            "text/plain; version=0.0.4; charset=utf-8"
        );
    }

    #[test]
    fn exporter_content_type_returns_prometheus_exposition_format() {
        let registry = Arc::new(MetricsRegistry::new());
        let exporter = PrometheusExporter::new(registry);
        assert_eq!(
            exporter.content_type(),
            "text/plain; version=0.0.4; charset=utf-8"
        );
    }

    #[test]
    fn snapshot_renders_multiple_labeled_families_correctly() {
        let registry = Arc::new(MetricsRegistry::new());
        let interner = registry.interner();
        let labels = interner.label_set(&[("action_type", "http.request")]);

        registry
            .counter_labeled("nebula_action_executions_total", &labels)
            .inc_by(10);
        registry
            .counter_labeled("nebula_action_failures_total", &labels)
            .inc_by(2);

        let out = snapshot(&registry);
        assert!(
            out.contains("# TYPE nebula_action_executions_total counter"),
            "executions TYPE:\n{out}"
        );
        assert!(
            out.contains("# TYPE nebula_action_failures_total counter"),
            "failures TYPE:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_action_executions_total{action_type="http.request"} 10"#),
            "executions value:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_action_failures_total{action_type="http.request"} 2"#),
            "failures value:\n{out}"
        );
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
