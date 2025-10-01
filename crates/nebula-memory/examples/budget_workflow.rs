//! Example demonstrating memory budgeting for workflow execution
//!
//! This example shows how to use the budget module to control and limit
//! memory usage during workflow execution.

use std::sync::Arc;
use nebula_memory::budget::{BudgetConfig, MemoryBudget, create_budget, create_child_budget};

fn main() {
    println!("=== Memory Budget Example ===\n");

    // Create a budget for a workflow with 10MB limit
    let workflow_budget = create_budget("workflow-1", 10 * 1024 * 1024);

    println!("Created workflow budget: {}", workflow_budget.name());
    println!("  Limit: {} bytes", workflow_budget.limit());
    println!("  Used: {} bytes\n", workflow_budget.used());

    // Simulate step 1: allocate 2MB
    println!("Step 1: Allocating 2MB...");
    match workflow_budget.request_memory(2 * 1024 * 1024) {
        Ok(_) => {
            println!("  ✓ Allocated successfully");
            print_metrics(&workflow_budget);
        }
        Err(e) => println!("  ✗ Failed: {}", e),
    }

    // Create a child budget for a subtask with 5MB limit
    let subtask_budget = create_child_budget(
        "subtask-1",
        5 * 1024 * 1024,
        workflow_budget.clone()
    );

    println!("\nStep 2: Created child budget for subtask");
    println!("  Subtask limit: {} bytes", subtask_budget.limit());

    // Simulate subtask allocation
    println!("Step 3: Subtask allocating 3MB...");
    match subtask_budget.request_memory(3 * 1024 * 1024) {
        Ok(_) => {
            println!("  ✓ Allocated successfully");
            println!("  Parent usage: {} bytes", workflow_budget.used());
            println!("  Child usage: {} bytes", subtask_budget.used());
        }
        Err(e) => println!("  ✗ Failed: {}", e),
    }

    // Try to exceed the limit
    println!("\nStep 4: Trying to allocate 6MB (should fail)...");
    match workflow_budget.request_memory(6 * 1024 * 1024) {
        Ok(_) => println!("  ✓ Allocated (unexpected)"),
        Err(e) => println!("  ✗ Failed as expected: {}", e),
    }

    // Release memory
    println!("\nStep 5: Releasing 2MB from parent...");
    workflow_budget.release_memory(2 * 1024 * 1024);
    print_metrics(&workflow_budget);

    // Now we should be able to allocate
    println!("\nStep 6: Allocating 1MB (should succeed)...");
    match workflow_budget.request_memory(1 * 1024 * 1024) {
        Ok(_) => {
            println!("  ✓ Allocated successfully");
            print_metrics(&workflow_budget);
        }
        Err(e) => println!("  ✗ Failed: {}", e),
    }

    println!("\n=== Final Statistics ===");
    print_full_metrics(&workflow_budget);
}

fn print_metrics(budget: &Arc<MemoryBudget>) {
    let metrics = budget.metrics();
    println!("  State: {:?}", metrics.state);
    println!("  Used: {} / {} bytes", metrics.used, metrics.limit);
    println!("  Peak: {} bytes", metrics.peak);
}

fn print_full_metrics(budget: &Arc<MemoryBudget>) {
    let metrics = budget.metrics();
    println!("Budget: {}", budget.name());
    println!("  State: {:?}", metrics.state);
    println!("  Used: {} bytes", metrics.used);
    println!("  Limit: {} bytes", metrics.limit);
    println!("  Peak: {} bytes", metrics.peak);
    println!("  Successful allocations: {}", metrics.allocations);
    println!("  Failed allocations: {}", metrics.failures);
}
