//! Demonstrates common subscriber patterns with nebula-eventbus.
//!
//! This example shows:
//! 1. Basic subscribe + recv loop
//! 2. Filtered subscription with custom predicates
//! 3. Lag monitoring with lagged_count()
//! 4. Multi-bus registry pattern

use nebula_eventbus::{BackPressurePolicy, EventBus, EventBusRegistry, EventFilter};
use std::sync::Arc;
use tokio::task;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowEvent {
    workflow_id: u64,
    status: String,
}

#[tokio::main]
async fn main() {
    println!("=== EventBus Subscriber Patterns ===\n");

    basic_subscribe_loop().await;
    println!();

    filtered_subscription().await;
    println!();

    lag_monitoring().await;
    println!();

    multi_bus_registry().await;
}

/// Pattern 1: Basic subscription loop
async fn basic_subscribe_loop() {
    println!("Pattern 1: Basic Subscribe + Recv Loop");

    let bus = EventBus::new(64);

    // Spawn emitter
    let bus_clone = bus.clone();
    let _emitter = task::spawn(async move {
        for i in 0..5 {
            let event = WorkflowEvent {
                workflow_id: i,
                status: "started".to_string(),
            };
            let outcome = bus_clone.emit(event);
            println!("  Emitted event {}: {:?}", i, outcome);
        }
    });

    // Subscribe and receive
    let mut sub = bus.subscribe();
    while let Some(event) = sub.recv().await {
        println!("  Received: workflow_id={}, status={}", event.workflow_id, event.status);
    }

    println!("  ✓ All events received");
}

/// Pattern 2: Filtered subscription
async fn filtered_subscription() {
    println!("Pattern 2: Filtered Subscription");

    let bus = Arc::new(EventBus::new(64));

    // Spawn emitter with mixed events
    let bus_clone = bus.clone();
    let _emitter = task::spawn(async move {
        for i in 0..10 {
            let status = if i % 2 == 0 { "started" } else { "completed" };
            let event = WorkflowEvent {
                workflow_id: i,
                status: status.to_string(),
            };
            let _outcome = bus_clone.emit(event);
        }
    });

    // Subscribe with filter: only "completed" events
    let filter = EventFilter::custom(|event: &WorkflowEvent| event.status == "completed");
    let mut filtered_sub = bus.subscribe_filtered(filter);

    let mut count = 0;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    while let Some(event) = filtered_sub.try_recv() {
        println!("  Received filtered event: workflow_id={}, status={}", event.workflow_id, event.status);
        count += 1;
    }
    println!("  ✓ Received {} filtered events", count);
}

/// Pattern 3: Lag monitoring
async fn lag_monitoring() {
    println!("Pattern 3: Lag Monitoring");

    let bus = Arc::new(EventBus::new(16)); // Small buffer to trigger lag
    let bus_clone = bus.clone();

    // Spawn fast emitter
    let _emitter = task::spawn(async move {
        for i in 0..100 {
            let event = WorkflowEvent {
                workflow_id: i,
                status: "started".to_string(),
            };
            let _outcome = bus_clone.emit(event);
        }
    });

    // Slow subscriber (sleeps between receiveifs)
    let mut sub = bus.subscribe();
    let mut received = 0;

    for _ in 0..50 {
        if let Some(_event) = sub.try_recv() {
            received += 1;
            let lagged = sub.lagged_count();
            if lagged > 0 {
                println!("  Received event #{}, lagged_count={}", received, lagged);
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
    }

    println!("  ✓ Received {} events (total lag: {})", received, sub.lagged_count());
}

/// Pattern 4: Multi-bus registry (per-tenant isolation)
async fn multi_bus_registry() {
    println!("Pattern 4: Multi-Bus Registry (Per-Tenant)");

    let registry: EventBusRegistry<String, WorkflowEvent> =
        EventBusRegistry::with_policy(64, BackPressurePolicy::DropOldest);

    // Create buses for different tenants
    let bus_tenant_a = registry.get_or_create("tenant_a".to_string());
    let bus_tenant_b = registry.get_or_create("tenant_b".to_string());

    // Emit events on different buses
    let _a = bus_tenant_a.emit(WorkflowEvent {
        workflow_id: 1,
        status: "tenant_a_event".to_string(),
    });
    let _b = bus_tenant_b.emit(WorkflowEvent {
        workflow_id: 2,
        status: "tenant_b_event".to_string(),
    });

    // Each tenant has isolated subscribers
    let mut sub_a = bus_tenant_a.subscribe();
    let mut sub_b = bus_tenant_b.subscribe();

    if let Some(event_a) = sub_a.try_recv() {
        println!("  Tenant A received: {}", event_a.status);
    }
    if let Some(event_b) = sub_b.try_recv() {
        println!("  Tenant B received: {}", event_b.status);
    }

    let stats = registry.stats();
    println!("  Registry stats: {} buses, {} sent events", stats.bus_count, stats.sent_count);
    println!("  ✓ Multi-tenant isolation working");
}
