//! # Prometheus Export Example
//!
//! Shows how to use `MetricsRegistry` with canonical `nebula_*` metric names
//! and render the registry to Prometheus text format.
//!
//! Run with:
//! ```bash
//! cargo run -p nebula-metrics --example prometheus_export
//! ```

use std::sync::Arc;

use nebula_metrics::export::prometheus;
use nebula_metrics::naming::{
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_DURATION_SECONDS, NEBULA_ACTION_FAILURES_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL, NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
};
use nebula_telemetry::metrics::MetricsRegistry;

fn main() {
    let registry = Arc::new(MetricsRegistry::new());

    // ── Simulate workflow executions ──────────────────────────────────────────

    registry
        .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
        .inc_by(5);
    registry
        .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
        .inc_by(4);
    registry
        .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
        .inc();

    for &secs in &[0.12, 0.35, 0.78, 1.45, 3.20] {
        registry
            .histogram(NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS)
            .observe(secs);
    }

    // ── Simulate action executions with labels ────────────────────────────────

    let interner = registry.interner();

    let http_ok = interner.label_set(&[("action_type", "http.request"), ("status", "success")]);
    let http_err = interner.label_set(&[("action_type", "http.request"), ("status", "error")]);
    let math_ok = interner.label_set(&[("action_type", "math.add"), ("status", "success")]);

    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &http_ok)
        .inc_by(120);
    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &http_err)
        .inc_by(8);
    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &math_ok)
        .inc_by(55);

    registry
        .counter_labeled(NEBULA_ACTION_FAILURES_TOTAL, &http_err)
        .inc_by(8);

    for &ms in &[45.0_f64, 112.0, 230.0, 890.0, 2100.0, 4500.0] {
        registry
            .histogram_labeled(NEBULA_ACTION_DURATION_SECONDS, &http_ok)
            .observe(ms / 1000.0);
    }
    for &ms in &[15.0_f64, 20.0, 18.0] {
        registry
            .histogram_labeled(NEBULA_ACTION_DURATION_SECONDS, &math_ok)
            .observe(ms / 1000.0);
    }

    // ── Render to Prometheus text format ──────────────────────────────────────

    let output = prometheus::snapshot(&registry);
    println!("{output}");

    // ── Content-type header ───────────────────────────────────────────────────
    println!("# Content-Type: {}", prometheus::content_type());
}
