//! Prometheus text format exporter.
//!
//! Renders metrics from a telemetry registry to Prometheus exposition format.
//! Use with an HTTP server to serve GET /metrics.
//!
//! The exporter iterates all entries in the registry (unlabeled **and**
//! labeled) via the `snapshot_*` APIs, groups them by metric name, and
//! renders each family with a single `# HELP` / `# TYPE` header followed by
//! sample lines. Labels are rendered as `{key1="value1",key2="value2"}`.

use std::{
    collections::{BTreeMap, HashMap, HashSet, hash_map::DefaultHasher},
    fmt::Write as _,
    hash::{Hash, Hasher},
    sync::Arc,
};

use nebula_telemetry::{labels::LabelInterner, metrics::MetricsRegistry};

use crate::naming::{
    NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, NEBULA_ACTION_DURATION_SECONDS,
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL, NEBULA_CACHE_EVICTIONS,
    NEBULA_CACHE_HITS, NEBULA_CACHE_MISSES, NEBULA_CACHE_SIZE, NEBULA_CREDENTIAL_ACTIVE_TOTAL,
    NEBULA_CREDENTIAL_EXPIRED_TOTAL, NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
    NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL, NEBULA_CREDENTIAL_ROTATIONS_TOTAL,
    NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, NEBULA_ENGINE_LEASE_CONTENTION_TOTAL,
    NEBULA_EVENTBUS_DROP_RATIO_PPM, NEBULA_EVENTBUS_DROPPED, NEBULA_EVENTBUS_SENT,
    NEBULA_EVENTBUS_SUBSCRIBERS, NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_CLEANUP_TOTAL, NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
    NEBULA_RESOURCE_DESTROY_TOTAL, NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS, NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};

/// Prometheus exposition format version (text-based).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

// ── Static metric descriptors ─────────────────────────────────────────────────

fn counter_help(name: &str) -> &'static str {
    match name {
        NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL => "Total workflow executions started.",
        NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL => {
            "Total workflow executions completed successfully."
        },
        NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL => "Total workflow executions failed.",
        NEBULA_ACTION_EXECUTIONS_TOTAL => "Total action executions.",
        NEBULA_ACTION_FAILURES_TOTAL => "Total action failures.",
        NEBULA_ACTION_DISPATCH_REJECTED_TOTAL => {
            "Total action dispatches rejected before reaching a handler."
        },
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
        NEBULA_RESOURCE_DESTROY_TOTAL => "Total resource instances destroyed.",
        NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL => "Total resource acquire errors.",
        NEBULA_CREDENTIAL_ROTATIONS_TOTAL => "Total credential rotation attempts.",
        NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL => "Total credential rotation failures.",
        NEBULA_CREDENTIAL_EXPIRED_TOTAL => "Total credentials expired.",
        NEBULA_ENGINE_LEASE_CONTENTION_TOTAL => "Total engine execution-lease contention events.",
        NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL => {
            "Total control-queue reclaim sweep outcomes (ADR-0017)."
        },
        _ => "Custom counter.",
    }
}

fn gauge_help(name: &str) -> &'static str {
    match name {
        NEBULA_RESOURCE_HEALTH_STATE => {
            "Resource health state (1=healthy, 0.5=degraded, 0=unhealthy)."
        },
        NEBULA_RESOURCE_POOL_WAITERS => "Number of waiters when pool exhausted.",
        NEBULA_EVENTBUS_SENT => "EventBus sent events snapshot.",
        NEBULA_EVENTBUS_DROPPED => "EventBus dropped events snapshot.",
        NEBULA_EVENTBUS_SUBSCRIBERS => "EventBus active subscribers snapshot.",
        NEBULA_EVENTBUS_DROP_RATIO_PPM => "EventBus drop ratio in parts-per-million.",
        NEBULA_CREDENTIAL_ACTIVE_TOTAL => "Number of active credentials.",
        NEBULA_CACHE_HITS => "Cache hits snapshot.",
        NEBULA_CACHE_MISSES => "Cache misses snapshot.",
        NEBULA_CACHE_EVICTIONS => "Cache evictions snapshot.",
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
        },
        NEBULA_RESOURCE_USAGE_DURATION_SECONDS => "Resource usage duration in seconds.",
        NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS => "Credential rotation duration in seconds.",
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
    let mut used_keys = HashSet::<String>::new();
    let mut out = String::from("{");
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let raw_k = interner.resolve(k);
        let v_str = interner.resolve(v);
        // Sanitize then ensure uniqueness: distinct raw keys can map to the same
        // identifier (e.g. "a-b" and "a b" → "a_b"), which would break Prometheus text format.
        let base = sanitize_label_key(raw_k);
        let mut key_out = base.clone();
        if !used_keys.insert(key_out.clone()) {
            let h = hash_raw(raw_k);
            key_out = format!("{base}__{h:016x}");
            while !used_keys.insert(key_out.clone()) {
                key_out.push('_');
            }
        }
        let v_escaped = escape_label_value(v_str);
        let _ = write!(out, "{key_out}=\"{v_escaped}\"");
    }
    out.push('}');
    out
}

fn sanitize_metric_name(name: &str) -> String {
    sanitize_identifier(name, true)
}

fn sanitize_label_key(key: &str) -> String {
    sanitize_identifier(key, false)
}

fn sanitize_identifier(input: &str, allow_colon: bool) -> String {
    if input.is_empty() {
        return "_".to_owned();
    }

    let mut out = String::with_capacity(input.len());
    for (index, ch) in input.chars().enumerate() {
        let is_valid = if index == 0 {
            ch.is_ascii_alphabetic() || ch == '_' || (allow_colon && ch == ':')
        } else {
            ch.is_ascii_alphanumeric() || ch == '_' || (allow_colon && ch == ':')
        };
        out.push(if is_valid { ch } else { '_' });
    }

    out
}

/// Stable hash for disambiguating colliding sanitized identifiers (label keys / metric names).
fn hash_raw(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Assign a unique exported metric name for each distinct raw name. If sanitization maps two
/// different raw names to the same string, the second and later names get a `__{hash}` suffix
/// so families do not merge and `# TYPE` lines stay valid.
fn allocate_exported_metric_name(
    raw: &str,
    raw_to_exported: &mut HashMap<String, String>,
    taken: &mut HashSet<String>,
) -> String {
    if let Some(existing) = raw_to_exported.get(raw) {
        return existing.clone();
    }
    let base = sanitize_metric_name(raw);
    let mut exported = base.clone();
    if taken.contains(&exported) {
        let h = hash_raw(raw);
        exported = format!("{base}__{h:016x}");
        while taken.contains(&exported) {
            exported.push('_');
        }
    }
    taken.insert(exported.clone());
    raw_to_exported.insert(raw.to_owned(), exported.clone());
    exported
}

fn escape_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped
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

    // One exported name per raw metric name string; disambiguate when sanitization collides
    // (Copilot review: merged families / duplicate `# TYPE` for the same exported name).
    let mut metric_raw_to_exported: HashMap<String, String> = HashMap::new();
    let mut exported_metric_names: HashSet<String> = HashSet::new();

    // ── Counters ──────────────────────────────────────────────────────────
    // Group by metric name so each family emits one HELP+TYPE header.
    let mut counter_families: BTreeMap<String, Vec<_>> = BTreeMap::new();
    for (key, counter) in registry.snapshot_counters() {
        let raw_name = interner.resolve(key.name);
        let name = allocate_exported_metric_name(
            raw_name,
            &mut metric_raw_to_exported,
            &mut exported_metric_names,
        );
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
        let raw_name = interner.resolve(key.name);
        let name = allocate_exported_metric_name(
            raw_name,
            &mut metric_raw_to_exported,
            &mut exported_metric_names,
        );
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
        let raw_name = interner.resolve(key.name);
        let name = allocate_exported_metric_name(
            raw_name,
            &mut metric_raw_to_exported,
            &mut exported_metric_names,
        );
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
            let buckets = hist.buckets();
            let label_str = render_labels(labels, interner);
            // Emit finite buckets using this histogram's configured boundaries.
            for (upper_bound, cumulative) in buckets.iter().filter(|(upper, _)| upper.is_finite()) {
                let le = upper_bound.to_string();
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

    #[test]
    fn snapshot_uses_histogram_specific_bucket_boundaries() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry.interner().label_set(&[("kind", "custom")]);
        let histogram = registry.histogram_with_buckets_labeled(
            "nebula_custom_duration_seconds",
            &labels,
            vec![0.1, 0.5],
        );
        histogram.observe(0.03);
        histogram.observe(0.3);
        histogram.observe(1.2);

        let out = snapshot(&registry);
        assert!(
            out.contains(r#"nebula_custom_duration_seconds_bucket{kind="custom",le="0.1"} 1"#),
            "expected first custom bucket count:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_custom_duration_seconds_bucket{kind="custom",le="0.5"} 2"#),
            "expected second custom bucket count:\n{out}"
        );
        assert!(
            out.contains(r#"nebula_custom_duration_seconds_bucket{kind="custom",le="+Inf"} 3"#),
            "expected +Inf bucket count:\n{out}"
        );
        assert!(
            !out.contains(r#"nebula_custom_duration_seconds_bucket{kind="custom",le="0.005"}"#),
            "default buckets should not be emitted for custom histogram:\n{out}"
        );
    }

    #[test]
    fn snapshot_sanitizes_metric_names_and_label_keys() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry
            .interner()
            .label_set(&[("bad key\nx", "a\"b\\c"), ("ok_key", "ok")]);
        registry
            .counter_labeled("bad metric\nname", &labels)
            .inc_by(2);

        let out = snapshot(&registry);
        assert!(
            out.contains("# TYPE bad_metric_name counter"),
            "metric name should be sanitized:\n{out}"
        );
        assert!(
            out.contains(r#"bad_metric_name{bad_key_x="a\"b\\c",ok_key="ok"} 2"#),
            "label keys should be sanitized and values escaped:\n{out}"
        );
    }

    #[test]
    fn snapshot_disambiguates_sanitized_label_key_collisions() {
        let registry = Arc::new(MetricsRegistry::new());
        let labels = registry
            .interner()
            .label_set(&[("a-b", "dash"), ("a b", "space")]);
        registry
            .counter_labeled("nebula_collision_label_keys", &labels)
            .inc();

        let out = snapshot(&registry);
        assert!(
            out.contains(r#"a_b="dash""#),
            "first label key should use the sanitized base name:\n{out}"
        );
        assert!(
            out.contains("__") && out.contains(r#"a_b__"#),
            "second colliding key should be suffixed with a stable hash:\n{out}"
        );
        assert!(
            out.contains(r#"a_b__"#) && out.contains(r#"="space""#),
            "both values should be present with distinct keys:\n{out}"
        );
    }

    #[test]
    fn snapshot_disambiguates_sanitized_metric_name_collisions() {
        let registry = Arc::new(MetricsRegistry::new());
        registry.counter("dup x").inc();
        registry.counter("dup-x").inc_by(2);

        let out = snapshot(&registry);
        let type_lines = out.lines().filter(|l| l.starts_with("# TYPE ")).count();
        assert_eq!(
            type_lines, 2,
            "expected two metric families when raw names collide after sanitization:\n{out}"
        );
        // Snapshot iteration order is not guaranteed; either raw name may claim the unsuffixed
        // `dup_x` family — values must still appear on two distinct exported names.
        assert!(
            out.lines()
                .any(|l| l.trim_end_matches('\r').ends_with(" 1")),
            "expected a sample line ending with 1:\n{out}"
        );
        assert!(
            out.lines()
                .any(|l| l.trim_end_matches('\r').ends_with(" 2")),
            "expected a sample line ending with 2:\n{out}"
        );
        assert!(
            out.contains("dup_x ") && out.contains("dup_x__"),
            "expected one base `dup_x` family and one `dup_x__{{hash}}` family:\n{out}"
        );
    }
}
