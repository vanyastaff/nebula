//! Multi-Crate Observability Example
//!
//! This example demonstrates how to collect observability data from multiple
//! nebula crates into a unified monitoring system.
//!
//! Shows:
//! - Unified event collection across crates
//! - Single Prometheus endpoint for all metrics
//! - Event correlation and tracing
//! - Centralized observability configuration

use nebula_log::observability::{ObservabilityEvent, ObservabilityHook, emit_event, register_hook};
use nebula_log::{info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize unified logging
    nebula_log::init()?;

    info!("Starting multi-crate observability example");

    println!("\n=== Multi-Crate Observability Setup ===\n");

    // Register a unified metrics collector
    let collector = Arc::new(UnifiedMetricsCollector::new());
    register_hook(collector.clone());

    // Simulate events from different nebula crates
    simulate_memory_crate_events();
    simulate_validator_crate_events();
    simulate_resource_crate_events();

    // Display collected metrics
    println!("\n=== Collected Metrics ===");
    collector.display_metrics();

    println!("\n=== Unified Prometheus Endpoint ===");
    println!("All metrics would be exported from a single endpoint:");
    println!("  http://localhost:9090/metrics\n");

    println!("Example unified metrics:");
    println!("  nebula_memory_cache_hits_total{{policy=\"lru\"}} 150");
    println!("  nebula_memory_cache_misses_total{{policy=\"lru\"}} 50");
    println!("  nebula_validator_validations_total{{type=\"field\"}} 42");
    println!("  nebula_resource_connections_active{{type=\"database\"}} 5");

    println!("\n=== Event Correlation ===");
    println!("Trace ID: abc-123");
    println!("  └─ nebula-memory: cache lookup (2ms)");
    println!("     └─ nebula-validator: validate input (5ms)");
    println!("        └─ nebula-resource: database query (25ms)");

    Ok(())
}

// ============================================================================
// Simulate Events from Different Crates
// ============================================================================

fn simulate_memory_crate_events() {
    info!(crate = "nebula-memory", "Simulating memory cache events");

    for i in 0..3 {
        let event = CrateEvent {
            crate_name: "nebula-memory".to_string(),
            event_type: "cache_operation".to_string(),
            operation: format!("lookup_{}", i),
            success: i % 2 == 0,
        };
        emit_event(&event);
    }
}

fn simulate_validator_crate_events() {
    info!(crate = "nebula-validator", "Simulating validation events");

    for i in 0..2 {
        let event = CrateEvent {
            crate_name: "nebula-validator".to_string(),
            event_type: "validation".to_string(),
            operation: format!("validate_{}", i),
            success: true,
        };
        emit_event(&event);
    }
}

fn simulate_resource_crate_events() {
    info!(crate = "nebula-resource", "Simulating resource events");

    let event = CrateEvent {
        crate_name: "nebula-resource".to_string(),
        event_type: "connection".to_string(),
        operation: "database_connect".to_string(),
        success: true,
    };
    emit_event(&event);
}

// ============================================================================
// Unified Metrics Collector
// ============================================================================

struct UnifiedMetricsCollector {
    metrics: Mutex<HashMap<String, CrateMetrics>>,
}

#[derive(Default)]
struct CrateMetrics {
    total_events: u64,
    success_count: u64,
    failure_count: u64,
}

impl UnifiedMetricsCollector {
    fn new() -> Self {
        Self {
            metrics: Mutex::new(HashMap::new()),
        }
    }

    fn display_metrics(&self) {
        let metrics = self.metrics.lock().unwrap();

        for (crate_name, crate_metrics) in metrics.iter() {
            println!("\n{}", crate_name);
            println!("  Total events:   {}", crate_metrics.total_events);
            println!("  Successful:     {}", crate_metrics.success_count);
            println!("  Failed:         {}", crate_metrics.failure_count);
            println!(
                "  Success rate:   {:.1}%",
                (crate_metrics.success_count as f64 / crate_metrics.total_events as f64) * 100.0
            );
        }
    }
}

impl ObservabilityHook for UnifiedMetricsCollector {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        if let Some(data) = event.data() {
            if let (Some(crate_name), Some(success)) = (
                data.get("crate_name").and_then(|v| v.as_str()),
                data.get("success").and_then(|v| v.as_bool()),
            ) {
                let mut metrics = self.metrics.lock().unwrap();
                let crate_metrics = metrics
                    .entry(crate_name.to_string())
                    .or_insert_with(CrateMetrics::default);

                crate_metrics.total_events += 1;
                if success {
                    crate_metrics.success_count += 1;
                } else {
                    crate_metrics.failure_count += 1;
                }
            }
        }
    }
}

// ============================================================================
// Event Types
// ============================================================================

struct CrateEvent {
    crate_name: String,
    event_type: String,
    operation: String,
    success: bool,
}

impl ObservabilityEvent for CrateEvent {
    fn name(&self) -> &str {
        "crate_event"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "crate_name": self.crate_name,
            "event_type": self.event_type,
            "operation": self.operation,
            "success": self.success,
        }))
    }
}
