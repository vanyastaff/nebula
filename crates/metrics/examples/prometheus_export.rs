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
    ACTION_DURATION, ACTION_EXECUTIONS, ACTION_FAILURES, WORKFLOW_EXECUTION_DURATION,
    WORKFLOW_EXECUTIONS_COMPLETED, WORKFLOW_EXECUTIONS_FAILED, WORKFLOW_EXECUTIONS_STARTED,
};
use nebula_telemetry::metrics::MetricsRegistry;

fn main() {
    let registry = Arc::new(MetricsRegistry::new());

    // ── Simulate workflow executions ──────────────────────────────────────────

    registry
        .counter(WORKFLOW_EXECUTIONS_STARTED.as_str())
        .inc_by(5);
    registry
        .counter(WORKFLOW_EXECUTIONS_COMPLETED.as_str())
        .inc_by(4);
    registry.counter(WORKFLOW_EXECUTIONS_FAILED.as_str()).inc();

    for &secs in &[0.12, 0.35, 0.78, 1.45, 3.20] {
        registry
            .histogram(WORKFLOW_EXECUTION_DURATION.as_str())
            .observe(secs);
    }

    // ── Simulate action executions with labels ────────────────────────────────

    let interner = registry.interner();

    let http_ok = interner.label_set(&[("action_type", "http.request"), ("status", "success")]);
    let http_err = interner.label_set(&[("action_type", "http.request"), ("status", "error")]);
    let math_ok = interner.label_set(&[("action_type", "math.add"), ("status", "success")]);

    registry
        .counter_labeled(ACTION_EXECUTIONS.as_str(), &http_ok)
        .inc_by(120);
    registry
        .counter_labeled(ACTION_EXECUTIONS.as_str(), &http_err)
        .inc_by(8);
    registry
        .counter_labeled(ACTION_EXECUTIONS.as_str(), &math_ok)
        .inc_by(55);

    registry
        .counter_labeled(ACTION_FAILURES.as_str(), &http_err)
        .inc_by(8);

    for &ms in &[45.0_f64, 112.0, 230.0, 890.0, 2100.0, 4500.0] {
        registry
            .histogram_labeled(ACTION_DURATION.as_str(), &http_ok)
            .observe(ms / 1000.0);
    }
    for &ms in &[15.0_f64, 20.0, 18.0] {
        registry
            .histogram_labeled(ACTION_DURATION.as_str(), &math_ok)
            .observe(ms / 1000.0);
    }

    // ── Render to Prometheus text format ──────────────────────────────────────

    let output = prometheus::snapshot(&registry);
    println!("{output}");

    // ── Content-type header ───────────────────────────────────────────────────
    println!("# Content-Type: {}", prometheus::content_type());
}
