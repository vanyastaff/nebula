//! Memory budgeting with per-context limits and hierarchy
//!
//! Demonstrates `MemoryBudget` — enforce memory limits per execution context,
//! with parent-child hierarchies for nested workflows.

use nebula_memory::budget::{BudgetConfig, MemoryBudget, create_budget, create_child_budget};

fn main() {
    // === Example 1: Basic budget ===
    println!("=== 1. Basic budget enforcement ===");

    let budget = create_budget("workflow-123", 1024);
    println!(
        "State: {:?}, Used: {}/{}",
        budget.state(),
        budget.used(),
        budget.limit()
    );

    // Request memory within budget
    budget.request_memory(256).unwrap();
    budget.request_memory(256).unwrap();
    println!(
        "After 512 bytes: state={:?}, used={}",
        budget.state(),
        budget.used()
    );

    // Release some
    budget.release_memory(128);
    println!("After release 128: used={}", budget.used());

    // Request more than available
    match budget.request_memory(2048) {
        Err(e) => println!("Over budget (expected): {e}"),
        Ok(()) => unreachable!(),
    }

    // === Example 2: Budget state transitions ===
    println!("\n=== 2. Incremental allocation ===");

    let budget = create_budget("state-demo", 1000);

    // Allocate incrementally and observe state
    for chunk in [200, 200, 200, 200, 200] {
        budget.request_memory(chunk).unwrap();
        let s = budget.state();
        println!(
            "  Used {:>4}/{}: {:?} (healthy={})",
            budget.used(),
            budget.limit(),
            s,
            s.is_healthy()
        );
    }

    // === Example 3: Parent-child hierarchy ===
    println!("\n=== 3. Parent-child hierarchy ===");

    let parent = create_budget("workflow", 4096);
    let child_a = create_child_budget("step-A", 2048, parent.clone());
    let child_b = create_child_budget("step-B", 2048, parent.clone());

    child_a.request_memory(1024).unwrap();
    child_b.request_memory(512).unwrap();

    println!("Parent used:  {}", parent.used());
    println!("Child A used: {}", child_a.used());
    println!("Child B used: {}", child_b.used());

    // Child release propagates to parent
    child_a.release_memory(512);
    println!("After child A releases 512: parent used={}", parent.used());

    // === Example 4: Check before allocate ===
    println!("\n=== 4. Pre-allocation check ===");

    let budget = create_budget("check-demo", 256);

    let sizes = [64, 128, 64, 128];
    for &size in &sizes {
        if budget.can_allocate(size) {
            budget.request_memory(size).unwrap();
            println!("Allocated {size} bytes (used={})", budget.used());
        } else {
            println!(
                "Cannot allocate {size} bytes (used={}, limit={})",
                budget.used(),
                budget.limit()
            );
        }
    }

    // === Example 5: Custom config ===
    println!("\n=== 5. Custom BudgetConfig ===");

    let config = BudgetConfig::new("custom", 2048)
        .with_min_guaranteed(256)
        .with_stats(true);

    let budget = MemoryBudget::new(config);
    budget.request_memory(512).unwrap();

    let metrics = budget.metrics();
    println!(
        "Metrics: used={}, peak={}, allocations={}, state={:?}",
        metrics.used, metrics.peak, metrics.allocations, metrics.state
    );
}
