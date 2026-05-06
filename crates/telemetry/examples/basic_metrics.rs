//! # Basic Metrics Example
//!
//! Demonstrates the core telemetry primitives:
//! - Unlabeled and labeled counters, gauges, and histograms
//! - String interning via `LabelInterner`
//! - Snapshot iteration for export
//!
//! Run with:
//! ```bash
//! cargo run -p nebula-telemetry --example basic_metrics
//! ```

use nebula_telemetry::metrics::MetricsRegistry;

fn main() {
    // ── Registry setup ────────────────────────────────────────────────────────
    let registry = MetricsRegistry::new();

    // ── Unlabeled metrics ─────────────────────────────────────────────────────

    // Each call to .counter() / .gauge() / .histogram() is idempotent:
    // the same underlying atomic is returned on repeat calls.
    let total_executions = registry
        .counter("executions_total")
        .expect("counter registration");
    total_executions.inc();
    total_executions.inc();
    total_executions.inc_by(10);
    println!("executions_total = {}", total_executions.get()); // 12

    let active_workers = registry
        .gauge("active_workers")
        .expect("gauge registration");
    active_workers.set(4);
    active_workers.inc();
    active_workers.dec();
    println!("active_workers   = {}", active_workers.get()); // 4

    let duration = registry
        .histogram("action_duration_seconds")
        .expect("histogram registration");
    for ms in [5, 12, 25, 100, 250, 1000] {
        duration.observe(ms as f64 / 1000.0);
    }
    println!(
        "duration p50={:.3}s  p99={:.3}s  sum={:.3}s  count={}",
        duration.percentile(0.50).unwrap_or(0.0),
        duration.percentile(0.99).unwrap_or(0.0),
        duration.sum(),
        duration.count(),
    );

    // ── Labeled metrics ───────────────────────────────────────────────────────

    // The interner is shared by the registry and cheaply cloneable.
    let interner = registry.interner();

    // LabelSets that differ only in insertion order hash and compare identically.
    let http_ok = interner.label_set(&[("action_type", "http.request"), ("status", "ok")]);
    let http_err = interner.label_set(&[("action_type", "http.request"), ("status", "error")]);
    let math_ok = interner.label_set(&[("action_type", "math.add"), ("status", "ok")]);

    registry
        .counter_labeled("action_executions_total", &http_ok)
        .expect("counter")
        .inc_by(42);
    registry
        .counter_labeled("action_executions_total", &http_err)
        .expect("counter")
        .inc_by(3);
    registry
        .counter_labeled("action_executions_total", &math_ok)
        .expect("counter")
        .inc_by(17);

    // Active concurrent actions per type (gauge).
    let http_active = interner.single("action_type", "http.request");
    registry
        .gauge_labeled("active_actions", &http_active)
        .expect("gauge")
        .set(5);

    // ── Snapshot / export ─────────────────────────────────────────────────────
    println!("\n── Counter snapshot ────────────────────────────────");
    let interner = registry.interner();
    for (key, counter) in registry.snapshot_counters() {
        let name = interner.resolve(key.name);
        let labels = key.labels.resolve(interner);
        if labels.is_empty() {
            println!("  {name} = {}", counter.get());
        } else {
            let label_str = labels
                .iter()
                .map(|(k, v)| format!("{k}=\"{v}\""))
                .collect::<Vec<_>>()
                .join(", ");
            println!("  {name}{{{label_str}}} = {}", counter.get());
        }
    }

    // ── last_updated_ms ───────────────────────────────────────────────────────
    println!("\n── Last updated timestamps ─────────────────────────");
    let c = registry
        .counter("executions_total")
        .expect("counter registration");
    println!(
        "  executions_total last_updated_ms = {}",
        c.last_updated_ms()
    );

    // ── metric_count ─────────────────────────────────────────────────────────
    println!(
        "\nTotal metric series in registry: {}",
        registry.metric_count()
    );
}
