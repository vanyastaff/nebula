//! # Cardinality Guard Example
//!
//! Demonstrates how to prevent registry bloat from high-cardinality labels
//! using two complementary mechanisms learned from Vector.dev and Linkerd:
//!
//! 1. **`LabelAllowlist`** — strips unsafe label keys (e.g. `execution_id`,
//!    `workflow_id`) before they reach the registry.
//! 2. **`MetricsRegistry::retain_recent`** — TTL-based eviction of metric series
//!    that have not been updated within a time window.
//!
//! Run with:
//! ```bash
//! cargo run -p nebula-metrics --example cardinality_guard
//! ```

use std::sync::Arc;
use std::time::Duration;

use nebula_metrics::filter::LabelAllowlist;
use nebula_metrics::naming::ACTION_EXECUTIONS;
use nebula_telemetry::metrics::MetricsRegistry;

fn main() {
    let registry = Arc::new(MetricsRegistry::new());

    // ── Step 1 — configure an allowlist ──────────────────────────────────────
    //
    // Only "safe" low-cardinality keys are allowed in Prometheus series.
    // Keys like `execution_id` or `workflow_id` are stripped automatically.
    let allowlist = LabelAllowlist::only(["action_type", "status"]);

    println!("=== LabelAllowlist demo ===\n");

    // Simulate an action executor that receives a full context label set.
    let interner = registry.interner();
    let raw_labels = interner.label_set(&[
        ("action_type", "http.request"),
        ("status", "success"),
        ("execution_id", "550e8400-e29b-41d4-a716-446655440000"), // high cardinality!
        ("workflow_id", "wf-123456"),                             // high cardinality!
    ]);

    println!("Raw labels ({} keys):", raw_labels.len());
    for (k, v) in raw_labels.resolve(interner) {
        println!("  {k} = {v}");
    }

    // Apply the allowlist — only "action_type" and "status" survive.
    let safe_labels = allowlist.apply(&raw_labels, interner);
    println!("\nFiltered labels ({} keys):", safe_labels.len());
    for (k, v) in safe_labels.resolve(interner) {
        println!("  {k} = {v}");
    }

    // Record with the safe set — no cardinality explosion.
    registry
        .counter_labeled(ACTION_EXECUTIONS.as_str(), &safe_labels)
        .inc_by(42);
    println!(
        "\naction_executions with safe labels = {}",
        registry
            .counter_labeled(ACTION_EXECUTIONS.as_str(), &safe_labels)
            .get()
    );

    println!("\n=== retain_recent demo ===\n");

    // ── Step 2 — simulate series accumulation then eviction ──────────────────
    //
    // Imagine 1 000 unique action label combinations were recorded in the past.
    // After a time window, only the currently active ones should remain.

    let reg2 = Arc::new(MetricsRegistry::new());
    let interner2 = reg2.interner();

    // Record 1 000 unique series (one per simulated plugin instance ID).
    for i in 0..1_000usize {
        let labels = interner2.label_set(&[("plugin_instance", i.to_string().as_str())]);
        reg2.counter_labeled("nebula_plugin_executions_total", &labels)
            .inc();
    }
    println!("Series after bulk recording : {}", reg2.metric_count());

    // A background task calls retain_recent on a schedule.
    // Anything not updated in the last 5 minutes will be evicted.
    // Here we use Duration::ZERO to force eviction of our stale series.
    std::thread::sleep(Duration::from_millis(2));
    reg2.retain_recent(Duration::ZERO);
    println!("Series after retain_recent  : {}", reg2.metric_count());

    // ── Step 3 — active series are retained ─────────────────────────────────
    let reg3 = Arc::new(MetricsRegistry::new());
    let interner3 = reg3.interner();

    let labels = interner3.label_set(&[("action_type", "http.request")]);
    reg3.counter_labeled("nebula_actions_total", &labels).inc();
    reg3.counter_labeled("nebula_actions_total", &labels).inc(); // keep-alive touch

    // With a generous window (5 min) fresh series survive.
    reg3.retain_recent(Duration::from_secs(300));
    println!(
        "\nActive series preserved by retain_recent: {}",
        reg3.metric_count()
    ); // 1

    println!("\n=== Summary ===");
    println!("LabelAllowlist strips unsafe keys at record-time (static guard)");
    println!("retain_recent evicts stale series at cleanup-time (dynamic guard)");
    println!("Use both together for full cardinality protection.");
}
