//! Global budget manager for centralized management of memory budgets
//!
//! This module provides a global budget manager that maintains a registry of
//! all memory budgets in the system and provides centralized control over
//! memory allocation and usage.

use std::collections::HashMap;
use std::sync::{Arc, Once};
use parking_lot::{Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::error::{MemoryError, MemoryResult};
use crate::stats::MemoryStats;
use super::budget::{MemoryBudget, BudgetState, BudgetMetrics};
use super::config::BudgetConfig;

/// Interface for budget managers
pub trait BudgetManager: Send + Sync {
    /// Get a budget by name
    fn get_budget(&self, name: &str) -> Option<Arc<MemoryBudget>>;
    
    /// Create a new budget
    fn create_budget(&self, config: BudgetConfig) -> Arc<MemoryBudget>;
    
    /// Create a child budget
    fn create_child_budget(
        &self,
        parent_name: &str,
        config: BudgetConfig,
    ) -> MemoryResult<Arc<MemoryBudget>>;
    
    /// Get all budgets
    fn get_all_budgets(&self) -> Vec<Arc<MemoryBudget>>;
    
    /// Get system-wide memory metrics
    fn system_metrics(&self) -> SystemMemoryMetrics;
}

/// System-wide memory metrics
#[derive(Debug, Clone)]
pub struct SystemMemoryMetrics {
    /// Total memory limit across all root budgets
    pub total_limit: usize,
    
    /// Total memory used across all budgets
    pub total_used: usize,
    
    /// Number of active budgets
    pub budget_count: usize,
    
    /// Number of budgets in critical state
    pub critical_budgets: usize,
    
    /// Timestamp of the metrics
    pub timestamp: Instant,
}

/// Global budget manager singleton
pub struct GlobalBudgetManager {
    /// Root budgets (no parent)
    root_budgets: RwLock<HashMap<String, Arc<MemoryBudget>>>,
    
    /// All budgets by name
    all_budgets: RwLock<HashMap<String, Arc<MemoryBudget>>>,
    
    /// Global configuration
    config: RwLock<BudgetConfig>,
    
    /// Last system metrics
    last_metrics: Mutex<Option<SystemMemoryMetrics>>,
    
    /// Metrics update interval
    metrics_interval: Duration,
    
    /// Last metrics update time
    last_update: Mutex<Instant>,
}

// Singleton implementation
static mut INSTANCE: Option<Arc<GlobalBudgetManager>> = None;
static INIT: Once = Once::new();

impl GlobalBudgetManager {
    /// Initialize the global budget manager
    pub fn initialize(config: BudgetConfig) -> MemoryResult<()> {
        if config.name != "global" {
            return Err(MemoryError::InvalidConfig {
                reason: "Global budget manager must have name 'global'".to_string(),
            });
        }
        
        INIT.call_once(|| {
            let manager = Arc::new(Self {
                root_budgets: RwLock::new(HashMap::new()),
                all_budgets: RwLock::new(HashMap::new()),
                config: RwLock::new(config),
                last_metrics: Mutex::new(None),
                metrics_interval: Duration::from_secs(1),
                last_update: Mutex::new(Instant::now()),
            });
            
            unsafe {
                INSTANCE = Some(manager);
            }
        });
        
        Ok(())
    }
    
    /// Get the global budget manager instance
    pub fn instance() -> Arc<GlobalBudgetManager> {
        unsafe {
            if let Some(instance) = &INSTANCE {
                instance.clone()
            } else {
                // Auto-initialize with default config if not already initialized
                let config = BudgetConfig::new("global", usize::MAX / 2);
                Self::initialize(config).expect("Failed to initialize global budget manager");
                INSTANCE.as_ref().unwrap().clone()
            }
        }
    }
    
    /// Register a budget with the manager
    fn register_budget(&self, budget: Arc<MemoryBudget>) {
        let name = budget.name();
        let mut all_budgets = self.all_budgets.write();
        
        // Only register if not already present
        if !all_budgets.contains_key(&name) {
            all_budgets.insert(name.clone(), budget.clone());
            
            // If this is a root budget, add to root_budgets as well
            if budget.parent().is_none() {
                let mut root_budgets = self.root_budgets.write();
                root_budgets.insert(name, budget);
            }
        }
    }
    
    /// Update system metrics
    fn update_metrics(&self) -> SystemMemoryMetrics {
        let mut last_update = self.last_update.lock();
        let now = Instant::now();
        
        // Only update if interval has passed
        if now.duration_since(*last_update) < self.metrics_interval {
            return self.last_metrics.lock().clone().unwrap_or_else(|| {
                // If no metrics exist yet, force an update
                drop(last_update);
                self.calculate_metrics()
            });
        }
        
        *last_update = now;
        let metrics = self.calculate_metrics();
        *self.last_metrics.lock() = Some(metrics.clone());
        
        metrics
    }
    
    /// Calculate system metrics
    fn calculate_metrics(&self) -> SystemMemoryMetrics {
        let root_budgets = self.root_budgets.read();
        let all_budgets = self.all_budgets.read();
        
        let total_limit: usize = root_budgets.values().map(|b| b.limit()).sum();
        let total_used: usize = root_budgets.values().map(|b| b.used()).sum();
        let budget_count = all_budgets.len();
        
        let critical_budgets = all_budgets
            .values()
            .filter(|b| b.state().is_critical())
            .count();
        
        SystemMemoryMetrics {
            total_limit,
            total_used,
            budget_count,
            critical_budgets,
            timestamp: Instant::now(),
        }
    }
}

impl BudgetManager for GlobalBudgetManager {
    fn get_budget(&self, name: &str) -> Option<Arc<MemoryBudget>> {
        self.all_budgets.read().get(name).cloned()
    }
    
    fn create_budget(&self, config: BudgetConfig) -> Arc<MemoryBudget> {
        let budget = MemoryBudget::new(config);
        self.register_budget(budget.clone());
        budget
    }
    
    fn create_child_budget(
        &self,
        parent_name: &str,
        config: BudgetConfig,
    ) -> MemoryResult<Arc<MemoryBudget>> {
        let parent = self.get_budget(parent_name).ok_or_else(|| {
            MemoryError::InvalidConfig {
                reason: format!("Parent budget '{}' not found", parent_name),
            }
        })?;
        
        let budget = MemoryBudget::with_parent(config, parent);
        self.register_budget(budget.clone());
        Ok(budget)
    }
    
    fn get_all_budgets(&self) -> Vec<Arc<MemoryBudget>> {
        self.all_budgets
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect()
    }
    
    fn system_metrics(&self) -> SystemMemoryMetrics {
        self.update_metrics()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    
    #[test]
    fn test_global_manager() {
        // Initialize with a test config
        let config = BudgetConfig::new("global", 1024 * 1024 * 1024); // 1GB
        GlobalBudgetManager::initialize(config).unwrap();
        
        let manager = GlobalBudgetManager::instance();
        
        // Create some budgets
        let budget1 = manager.create_budget(BudgetConfig::new("budget1", 1024 * 1024));
        let budget2 = manager.create_budget(BudgetConfig::new("budget2", 2 * 1024 * 1024));
        
        // Create a child budget
        let child = manager
            .create_child_budget("budget1", BudgetConfig::new("child", 512 * 1024))
            .unwrap();
        
        // Check that budgets are registered
        assert!(manager.get_budget("budget1").is_some());
        assert!(manager.get_budget("budget2").is_some());
        assert!(manager.get_budget("child").is_some());
        
        // Check budget relationships
        assert_eq!(child.parent().unwrap().name(), "budget1");
        
        // Check system metrics
        let metrics = manager.system_metrics();
        assert_eq!(metrics.budget_count, 3);
        assert_eq!(metrics.total_limit, 3 * 1024 * 1024); // budget1 + budget2
        assert_eq!(metrics.total_used, 0);
        assert_eq!(metrics.critical_budgets, 0);
        
        // Allocate some memory
        budget1.request_memory(512 * 1024).unwrap();
        child.request_memory(256 * 1024).unwrap();
        
        // Check updated metrics
        let metrics = manager.system_metrics();
        assert_eq!(metrics.total_used, 768 * 1024); // 512K + 256K
    }
    
    #[test]
    fn test_concurrent_access() {
        let manager = GlobalBudgetManager::instance();
        
        // Create a budget in the main thread
        let budget = manager.create_budget(BudgetConfig::new("concurrent_test", 1024 * 1024));
        
        // Spawn threads to access the budget
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let budget = budget.clone();
                thread::spawn(move || {
                    // Each thread allocates a different amount
                    let amount = (i + 1) * 10 * 1024;
                    budget.request_memory(amount).unwrap();
                    thread::sleep(Duration::from_millis(10));
                    budget.release_memory(amount / 2);
                })
            })
            .collect();
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
        
        // Check that the budget reflects all operations
        assert!(budget.used() > 0);
        assert!(budget.peak() > budget.used());
    }
}