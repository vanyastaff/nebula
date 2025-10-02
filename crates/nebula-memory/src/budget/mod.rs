//! Memory budgeting system for controlling and limiting memory usage
//!
//! This module provides a simple memory budgeting system for workflow execution.
//! It allows setting memory limits and tracking usage without complex DI/lifecycle dependencies.
//!
//! # Examples
//!
//! ```rust
//! use nebula_memory::budget::{BudgetConfig, MemoryBudget};
//!
//! // Create a budget for a workflow
//! let budget = MemoryBudget::new(BudgetConfig::new("workflow-1", 100 * 1024 * 1024));
//!
//! // Request memory allocation
//! if budget.request_memory(1024).is_ok() {
//!     // ... perform work ...
//!     budget.release_memory(1024);
//! }
//!
//! // Check metrics
//! let metrics = budget.metrics();
//! println!("Used: {} / {} bytes", metrics.used, metrics.limit);
//! ```

mod config;
mod budget;

pub use config::{BudgetConfig, OvercommitPolicy, ReservationMode};
pub use budget::{MemoryBudget, BudgetState, BudgetMetrics};

use std::sync::Arc;

/// Create a new memory budget with the given configuration
///
/// This is a convenience function for creating budgets without a parent.
pub fn create_budget(
    name: impl Into<String>,
    limit: usize,
) -> Arc<MemoryBudget> {
    let config = BudgetConfig::new(name, limit);
    MemoryBudget::new(config)
}

/// Create a new memory budget with a parent budget
///
/// Child budgets are constrained by both their own limit and their parent's remaining capacity.
pub fn create_child_budget(
    name: impl Into<String>,
    limit: usize,
    parent: Arc<MemoryBudget>,
) -> Arc<MemoryBudget> {
    let config = BudgetConfig::new(name, limit);
    MemoryBudget::with_parent(config, parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_creation() {
        let budget = create_budget("test", 1024 * 1024);
        assert_eq!(budget.name(), "test");
        assert_eq!(budget.limit(), 1024 * 1024);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn test_budget_hierarchy() {
        let parent = create_budget("parent", 1024 * 1024);
        let child = create_child_budget("child", 512 * 1024, parent.clone());

        assert_eq!(child.name(), "child");
        assert_eq!(child.limit(), 512 * 1024);
        assert_eq!(child.parent().unwrap().name(), "parent");
    }

    #[test]
    fn test_workflow_budget() {
        let workflow = create_budget("workflow-1", 10 * 1024 * 1024);

        // Simulate memory allocation pattern
        assert!(workflow.request_memory(1024 * 1024).is_ok());
        assert_eq!(workflow.used(), 1024 * 1024);
        assert!(workflow.state().is_healthy());

        // Release and check
        workflow.release_memory(512 * 1024);
        assert_eq!(workflow.used(), 512 * 1024);

        // Check metrics
        let metrics = workflow.metrics();
        assert_eq!(metrics.used, 512 * 1024);
        assert_eq!(metrics.limit, 10 * 1024 * 1024);
        assert_eq!(metrics.allocations, 1);
    }
}





