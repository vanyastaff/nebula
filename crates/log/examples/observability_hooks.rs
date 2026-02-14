//! Example demonstrating observability hooks system
//!
//! This example shows how to:
//! 1. Create custom events
//! 2. Create custom hooks
//! 3. Use built-in hooks (logging, metrics)
//! 4. Track operation lifecycle

use nebula_log::observability::{
    LoggingHook, ObservabilityEvent, ObservabilityHook, OperationCompleted, OperationFailed,
    OperationStarted, OperationTracker, emit_event, register_hook,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

// =============================================================================
// Custom Event Types
// =============================================================================

/// Custom event for validation
struct ValidationEvent {
    field: String,
    valid: bool,
    message: String,
}

impl ObservabilityEvent for ValidationEvent {
    fn name(&self) -> &str {
        "validation"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "field": self.field,
            "valid": self.valid,
            "message": self.message,
        }))
    }
}

/// Custom event for resource allocation
struct AllocationEvent {
    resource_type: String,
    size_bytes: usize,
}

impl ObservabilityEvent for AllocationEvent {
    fn name(&self) -> &str {
        "allocation"
    }

    fn data(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "resource_type": self.resource_type,
            "size_bytes": self.size_bytes,
        }))
    }
}

// =============================================================================
// Custom Hook Implementations
// =============================================================================

/// Hook that counts events by type
struct CountingHook {
    count: Arc<AtomicUsize>,
}

impl ObservabilityHook for CountingHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        self.count.fetch_add(1, Ordering::Relaxed);
        println!(
            "  [CountingHook] Event #{}: {}",
            self.count.load(Ordering::Relaxed),
            event.name()
        );
    }

    fn initialize(&self) {
        println!("  [CountingHook] Initialized");
    }

    fn shutdown(&self) {
        println!(
            "  [CountingHook] Shutdown (total events: {})",
            self.count.load(Ordering::Relaxed)
        );
    }
}

/// Hook that filters events and only processes specific types
struct FilteringHook {
    filter: String,
}

impl ObservabilityHook for FilteringHook {
    fn on_event(&self, event: &dyn ObservabilityEvent) {
        if event.name().contains(&self.filter) {
            println!("  [FilteringHook] Matched event: {}", event.name());
            if let Some(data) = event.data() {
                println!("    Data: {}", data);
            }
        }
    }

    fn initialize(&self) {
        println!("  [FilteringHook] Initialized (filter: {})", self.filter);
    }
}

// =============================================================================
// Demo Functions
// =============================================================================

fn demo_basic_events() {
    println!("\n=== Demo 1: Basic Events ===");

    // Emit operation lifecycle events
    let started = OperationStarted {
        operation: "fetch_user".to_string(),
        context: "api_handler".to_string(),
    };
    emit_event(&started);

    // Simulate work
    std::thread::sleep(Duration::from_millis(50));

    let completed = OperationCompleted {
        operation: "fetch_user".to_string(),
        duration: Duration::from_millis(50),
    };
    emit_event(&completed);

    // Emit a failed operation
    let failed = OperationFailed {
        operation: "update_user".to_string(),
        error: "database connection lost".to_string(),
        duration: Duration::from_millis(100),
    };
    emit_event(&failed);
}

fn demo_custom_events() {
    println!("\n=== Demo 2: Custom Events ===");

    // Validation event
    let validation = ValidationEvent {
        field: "email".to_string(),
        valid: false,
        message: "invalid format".to_string(),
    };
    emit_event(&validation);

    // Allocation event
    let allocation = AllocationEvent {
        resource_type: "buffer".to_string(),
        size_bytes: 4096,
    };
    emit_event(&allocation);
}

fn demo_operation_tracker() {
    println!("\n=== Demo 3: Operation Tracker (RAII) ===");

    // Success case
    {
        let tracker = OperationTracker::new("database_query", "list_users");
        std::thread::sleep(Duration::from_millis(25));
        tracker.success();
    }

    // Failure case
    {
        let tracker = OperationTracker::new("cache_lookup", "user_123");
        std::thread::sleep(Duration::from_millis(10));
        tracker.fail("cache miss");
    }

    // Dropped without completion (implicit failure)
    {
        let _tracker = OperationTracker::new("background_task", "cleanup");
        std::thread::sleep(Duration::from_millis(5));
        // Will emit failure on drop
    }
}

fn demo_metrics_integration() {
    println!("\n=== Demo 4: Metrics Integration ===");

    #[cfg(feature = "observability")]
    {
        use nebula_log::observability::MetricsHook;

        // Register metrics hook
        register_hook(Arc::new(MetricsHook::new()));
        println!("  Registered MetricsHook (events will be recorded as metrics)");
    }

    #[cfg(not(feature = "observability"))]
    {
        println!("  MetricsHook requires 'observability' feature");
    }

    // Emit events that will be recorded as metrics
    emit_event(&OperationStarted {
        operation: "api_request".to_string(),
        context: "handler".to_string(),
    });

    emit_event(&OperationCompleted {
        operation: "api_request".to_string(),
        duration: Duration::from_millis(42),
    });
}

// =============================================================================
// Main
// =============================================================================

fn main() {
    // Initialize logging
    nebula_log::init_with(nebula_log::Config::development()).expect("failed to initialize logging");

    println!("=== Nebula Observability Hooks Demo ===\n");

    // Register hooks
    println!("Registering hooks...");

    // Built-in logging hook
    register_hook(Arc::new(LoggingHook::new(tracing::Level::INFO)));

    // Custom counting hook
    let counter = Arc::new(AtomicUsize::new(0));
    register_hook(Arc::new(CountingHook {
        count: Arc::clone(&counter),
    }));

    // Custom filtering hook (only operations)
    register_hook(Arc::new(FilteringHook {
        filter: "operation".to_string(),
    }));

    println!();

    // Run demos
    demo_basic_events();
    demo_custom_events();
    demo_operation_tracker();
    demo_metrics_integration();

    // Show final stats
    println!("\n=== Final Stats ===");
    println!("Total events emitted: {}", counter.load(Ordering::Relaxed));

    // Shutdown hooks
    println!("\nShutting down hooks...");
    nebula_log::observability::shutdown_hooks();

    println!("\n=== Demo Complete ===");
}
