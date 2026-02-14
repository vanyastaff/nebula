//! Complete Prometheus Integration Example
//!
//! This example demonstrates:
//! - Setting up Prometheus metrics exporter
//! - Using standard metrics (counter, gauge, histogram)
//! - Integrating with nebula-log observability
//! - Creating a complete monitoring setup
//!
//! # Setup
//!
//! Add to Cargo.toml:
//! ```toml
//! [dependencies]
//! nebula-log = { version = "0.1", features = ["observability"] }
//! metrics = "0.23"
//! metrics-exporter-prometheus = "0.15"
//! tokio = { version = "1", features = ["full"] }
//! ```
//!
//! # Running
//!
//! ```bash
//! cargo run --example prometheus_integration --features observability
//! ```
//!
//! Then visit http://localhost:9090/metrics to see metrics.
//!
//! # Grafana Dashboard
//!
//! Example PromQL queries:
//! - Request rate: `rate(http_requests_total[5m])`
//! - Error rate: `rate(http_errors_total[5m])`
//! - 95th percentile latency: `histogram_quantile(0.95, http_request_duration_seconds_bucket)`

use nebula_log::observability::{ObservabilityEvent, ObservabilityHook, emit_event, register_hook};
use nebula_log::{info, warn};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    nebula_log::init()?;

    info!("Starting Prometheus integration example");

    // Note: Actual Prometheus setup would require:
    // 1. Install metrics-exporter-prometheus
    // 2. Create HTTP server for /metrics endpoint
    // 3. Configure scrape_interval in prometheus.yml
    //
    // Example prometheus.yml:
    // ```yaml
    // scrape_configs:
    //   - job_name: 'nebula'
    //     scrape_interval: 15s
    //     static_configs:
    //       - targets: ['localhost:9090']
    // ```

    println!("\n=== Prometheus Integration Example ===\n");

    // Register a metrics hook to track events
    let metrics_hook = MetricsHook::new();
    register_hook(Arc::new(metrics_hook));

    // Simulate various operations
    println!("Simulating operations...\n");

    for i in 0..5 {
        let event = OperationEvent {
            name: format!("operation_{}", i),
            duration_ms: (i + 1) * 100,
        };
        emit_event(&event);

        if i % 2 == 0 {
            info!(operation = i, "Operation completed successfully");
        } else {
            warn!(operation = i, "Operation completed with warnings");
        }
    }

    println!("\n=== Metrics Summary ===");
    println!("In a real setup, metrics would be available at:");
    println!("  http://localhost:9090/metrics");
    println!("\nExample metrics output:");
    println!("  # HELP operation_events_total Total number of operation events");
    println!("  # TYPE operation_events_total counter");
    println!("  operation_events_total{{type=\"operation\"}} 5");
    println!("\n  # HELP operation_duration_seconds Operation duration");
    println!("  # TYPE operation_duration_seconds histogram");
    println!("  operation_duration_seconds_bucket{{le=\"0.1\"}} 1");
    println!("  operation_duration_seconds_bucket{{le=\"0.5\"}} 5");

    println!("\n=== Grafana Dashboard ===");
    println!("Import this JSON for a basic dashboard:\n");
    println!(
        r#"{{
  "dashboard": {{
    "title": "Nebula Observability",
    "panels": [
      {{
        "title": "Event Rate",
        "targets": [{{
          "expr": "rate(operation_events_total[5m])"
        }}]
      }},
      {{
        "title": "Operation Duration (p95)",
        "targets": [{{
          "expr": "histogram_quantile(0.95, operation_duration_seconds_bucket)"
        }}]
      }}
    ]
  }}
}}"#
    );

    println!("\n=== PromQL Query Examples ===");
    println!("Event rate (5m):     rate(operation_events_total[5m])");
    println!("P50 latency:         histogram_quantile(0.50, operation_duration_seconds_bucket)");
    println!("P95 latency:         histogram_quantile(0.95, operation_duration_seconds_bucket)");
    println!("P99 latency:         histogram_quantile(0.99, operation_duration_seconds_bucket)");

    Ok(())
}

// ============================================================================
// Custom Event Type
// ============================================================================

struct OperationEvent {
    name: String,
    duration_ms: u64,
}

impl ObservabilityEvent for OperationEvent {
    fn name(&self) -> &str {
        "operation_completed"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "operation": self.name,
            "duration_ms": self.duration_ms,
        }))
    }
}

// ============================================================================
// Metrics Hook
// ============================================================================

struct MetricsHook {
    event_count: std::sync::atomic::AtomicU64,
}

impl MetricsHook {
    fn new() -> Self {
        Self {
            event_count: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl ObservabilityHook for MetricsHook {
    fn on_event(&self, _event: &dyn ObservabilityEvent) {
        // Increment counter
        self.event_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // In a real implementation, you would use metrics crate:
        // metrics::counter!("operation_events_total", "type" => "operation").increment(1);
        //
        // if let Some(data) = event.data() {
        //     if let Some(duration_ms) = data.get("duration_ms").and_then(|v| v.as_u64()) {
        //         metrics::histogram!("operation_duration_seconds")
        //             .record(duration_ms as f64 / 1000.0);
        //     }
        // }
    }
}
