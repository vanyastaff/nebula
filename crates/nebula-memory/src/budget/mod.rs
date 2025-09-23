

//! Memory budgeting system for controlling and limiting memory usage
//!
//! This module provides a comprehensive memory budgeting system that allows
//! for fine-grained control over memory allocation and usage across different
//! components of the system. It supports hierarchical budgets, priority-based
//! allocation, and adaptive behavior based on system memory pressure.

mod config;
mod budget;
mod manager;
mod policy;
mod reservation;

pub use config::{BudgetConfig, OvercommitPolicy, ReservationMode};
pub use budget::{MemoryBudget, BudgetState, BudgetMetrics};
pub use manager::{BudgetManager, GlobalBudgetManager};
pub use policy::{AllocationPolicy, PriorityPolicy, FairSharePolicy};
pub use reservation::{MemoryReservation, ReservationToken};

use std::sync::Arc;
use crate::traits::context::MemoryContext;
use crate::traits::isolation::{MemoryIsolation, MemoryIsolationResult, MemoryAllocation};
use crate::stats::MemoryStats;

/// Initialize the global budget manager with the given configuration
pub fn initialize(config: BudgetConfig) -> crate::error::MemoryResult<()> {
    manager::GlobalBudgetManager::initialize(config)
}

/// Get a reference to the global budget manager
pub fn global_manager() -> Arc<manager::GlobalBudgetManager> {
    manager::GlobalBudgetManager::instance()
}

/// Create a new memory budget with the given configuration
pub fn create_budget(
    name: impl Into<String>,
    limit: usize,
    parent: Option<Arc<MemoryBudget>>,
) -> Arc<MemoryBudget> {
    let config = BudgetConfig::new(name, limit);
    if let Some(parent) = parent {
        MemoryBudget::with_parent(config, parent)
    } else {
        MemoryBudget::new(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_budget_creation() {
        let budget = create_budget("test", 1024 * 1024, None);
        assert_eq!(budget.name(), "test");
        assert_eq!(budget.limit(), 1024 * 1024);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn test_budget_hierarchy() {
        let parent = create_budget("parent", 1024 * 1024, None);
        let child = create_budget("child", 512 * 1024, Some(parent.clone()));
        
        assert_eq!(child.name(), "child");
        assert_eq!(child.limit(), 512 * 1024);
        assert_eq!(child.parent().unwrap().name(), "parent");
    }
}
