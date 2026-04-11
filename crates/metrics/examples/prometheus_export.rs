//! # Prometheus Export Example
//!
//! Shows how to use `TelemetryAdapter` with canonical `nebula_*` metric names
//! and render the registry to Prometheus text format.
//!
//! Run with:
//! ```bash
//! cargo run -p nebula-metrics --example prometheus_export
//! ```

use std::sync::Arc;

use nebula_metrics::{adapter::TelemetryAdapter, export::prometheus};
use nebula_telemetry::metrics::MetricsRegistry;

fn main() {
    let registry = Arc::new(MetricsRegistry::new());
    let adapter = TelemetryAdapter::new(Arc::clone(&registry));

    // ── Simulate workflow executions ──────────────────────────────────────────

    adapter.workflow_executions_started_total().inc_by(5);
    adapter.workflow_executions_completed_total().inc_by(4);
    adapter.workflow_executions_failed_total().inc();

    for &secs in &[0.12, 0.35, 0.78, 1.45, 3.20] {
        adapter.workflow_execution_duration_seconds().observe(secs);
    }

    // ── Simulate action executions with labels ────────────────────────────────

    let interner = adapter.interner();

    let http_ok = interner.label_set(&[("action_type", "http.request"), ("status", "success")]);
    let http_err = interner.label_set(&[("action_type", "http.request"), ("status", "error")]);
    let math_ok = interner.label_set(&[("action_type", "math.add"), ("status", "success")]);

    adapter.action_executions_labeled(&http_ok).inc_by(120);
    adapter.action_executions_labeled(&http_err).inc_by(8);
    adapter.action_executions_labeled(&math_ok).inc_by(55);

    adapter.action_failures_labeled(&http_err).inc_by(8);

    for &ms in &[45.0_f64, 112.0, 230.0, 890.0, 2100.0, 4500.0] {
        adapter
            .action_duration_labeled(&http_ok)
            .observe(ms / 1000.0);
    }
    for &ms in &[15.0_f64, 20.0, 18.0] {
        adapter
            .action_duration_labeled(&math_ok)
            .observe(ms / 1000.0);
    }

    // ── Render to Prometheus text format ──────────────────────────────────────

    let output = prometheus::snapshot(&registry);
    println!("{output}");

    // ── Content-type header ───────────────────────────────────────────────────
    println!("# Content-Type: {}", prometheus::content_type());
}
