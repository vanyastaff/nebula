//! # Prometheus Export Example
//!
//! Records a handful of canonical `nebula_*` workflow / action metrics directly
//! through [`MetricsRegistry`] and renders the registry to Prometheus text
//! format via [`prometheus::snapshot`].
//!
//! Run with:
//! ```bash
//! cargo run -p nebula-metrics --example prometheus_export
//! ```

use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_ACTION_DURATION_SECONDS, NEBULA_ACTION_EXECUTIONS_TOTAL,
        NEBULA_ACTION_FAILURES_TOTAL, NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
        NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
        NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
    },
    prelude::snapshot,
};

fn main() {
    let registry = MetricsRegistry::new();

    // ── Simulate workflow executions ──────────────────────────────────────────

    registry
        .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
        .expect("workflow started counter")
        .inc_by(5);
    registry
        .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
        .expect("workflow completed counter")
        .inc_by(4);
    registry
        .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
        .expect("workflow failed counter")
        .inc();

    for &secs in &[0.12, 0.35, 0.78, 1.45, 3.20] {
        registry
            .histogram(NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS)
            .expect("workflow duration histogram")
            .observe(secs);
    }

    // ── Simulate action executions with labels ────────────────────────────────

    let interner = registry.interner();

    let http_ok = interner.label_set(&[("action_type", "http.request"), ("status", "success")]);
    let http_err = interner.label_set(&[("action_type", "http.request"), ("status", "error")]);
    let math_ok = interner.label_set(&[("action_type", "math.add"), ("status", "success")]);

    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &http_ok)
        .expect("action executions")
        .inc_by(120);
    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &http_err)
        .expect("action executions")
        .inc_by(8);
    registry
        .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, &math_ok)
        .expect("action executions")
        .inc_by(55);

    registry
        .counter_labeled(NEBULA_ACTION_FAILURES_TOTAL, &http_err)
        .expect("action failures")
        .inc_by(8);

    for &ms in &[45.0_f64, 112.0, 230.0, 890.0, 2100.0, 4500.0] {
        registry
            .histogram_labeled(NEBULA_ACTION_DURATION_SECONDS, &http_ok)
            .expect("action duration")
            .observe(ms / 1000.0);
    }
    for &ms in &[15.0_f64, 20.0, 18.0] {
        registry
            .histogram_labeled(NEBULA_ACTION_DURATION_SECONDS, &math_ok)
            .expect("action duration")
            .observe(ms / 1000.0);
    }

    // ── Render to Prometheus text format ──────────────────────────────────────

    let output = snapshot(&registry);
    println!("{output}");

    // ── Content-type header ───────────────────────────────────────────────────
    println!("# Content-Type: {}", nebula_metrics::content_type());
}
