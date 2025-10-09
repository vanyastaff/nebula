//! Core memory budget implementation
//!
//! This module provides the core implementation of the memory budgeting system,
//! including the MemoryBudget struct and related types.

use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use super::config::BudgetConfig;
use crate::error::{MemoryError, MemoryResult};

/// Current state of a memory budget
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetState {
    /// Normal operation, plenty of memory available
    Normal,

    /// High memory usage (>70% of limit)
    HighUsage,

    /// Critical memory usage (>90% of limit)
    Critical,

    /// Memory limit exceeded
    Exceeded,

    /// Budget is disabled
    Disabled,
}

impl BudgetState {
    /// Check if the budget is in a healthy state
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Normal | Self::HighUsage)
    }

    /// Check if the budget is in a critical state
    pub fn is_critical(&self) -> bool {
        matches!(self, Self::Critical | Self::Exceeded)
    }
}

/// Memory budget metrics
#[derive(Debug, Clone)]
pub struct BudgetMetrics {
    /// Current memory usage in bytes
    pub used: usize,

    /// Memory limit in bytes
    pub limit: usize,

    /// Peak memory usage in bytes
    pub peak: usize,

    /// Number of successful allocations
    pub allocations: usize,

    /// Number of failed allocations
    pub failures: usize,

    /// Current budget state
    pub state: BudgetState,

    /// Timestamp of the metrics
    pub timestamp: Instant,
}

/// Core memory budget implementation
pub struct MemoryBudget {
    /// Budget configuration
    config: RwLock<BudgetConfig>,

    /// Current memory usage in bytes
    used: Mutex<usize>,

    /// Peak memory usage in bytes
    peak: Mutex<usize>,

    /// Allocation statistics
    stats: Mutex<AllocationStats>,

    /// Parent budget (if any)
    parent: Option<Arc<MemoryBudget>>,

    /// Child budgets
    children: Mutex<Vec<Weak<MemoryBudget>>>,

    /// Usage history (if tracking is enabled)
    history: Mutex<Option<UsageHistory>>,
}

/// Statistics for memory allocations
#[derive(Debug, Default)]
struct AllocationStats {
    /// Number of successful allocations
    pub successful: usize,

    /// Number of failed allocations
    pub failed: usize,

    /// Total bytes allocated
    pub total_bytes: usize,

    /// Largest single allocation
    pub largest_allocation: usize,
}

/// History of memory usage
#[derive(Debug)]
struct UsageHistory {
    /// Timestamps of measurements
    pub timestamps: Vec<Instant>,

    /// Memory usage at each timestamp
    pub usage: Vec<usize>,

    /// Maximum number of entries to keep
    pub capacity: usize,

    /// Minimum time between entries
    pub min_interval: Duration,

    /// Last update time
    pub last_update: Instant,
}

impl UsageHistory {
    /// Create a new usage history
    fn new(capacity: usize, min_interval: Duration) -> Self {
        Self {
            timestamps: Vec::with_capacity(capacity),
            usage: Vec::with_capacity(capacity),
            capacity,
            min_interval,
            last_update: Instant::now(),
        }
    }

    /// Add a new usage data point
    fn add(&mut self, usage: usize) {
        let now = Instant::now();

        // Only add if minimum interval has passed
        if now.duration_since(self.last_update) < self.min_interval {
            return;
        }

        // If we've reached capacity, remove oldest entry
        if self.timestamps.len() >= self.capacity {
            self.timestamps.remove(0);
            self.usage.remove(0);
        }

        self.timestamps.push(now);
        self.usage.push(usage);
        self.last_update = now;
    }

    /// Get the average usage over the history
    fn average_usage(&self) -> Option<usize> {
        if self.usage.is_empty() {
            return None;
        }

        let sum: usize = self.usage.iter().sum();
        Some(sum / self.usage.len())
    }
}

impl MemoryBudget {
    /// Create a new memory budget with the given configuration
    pub fn new(config: BudgetConfig) -> Arc<Self> {
        let history = if config.tracking_window.is_some() && config.collect_stats {
            Some(UsageHistory::new(
                100,                        // Store 100 data points
                Duration::from_millis(100), // Minimum 100ms between points
            ))
        } else {
            None
        };

        Arc::new(Self {
            config: RwLock::new(config),
            used: Mutex::new(0),
            peak: Mutex::new(0),
            stats: Mutex::new(AllocationStats::default()),
            parent: None,
            children: Mutex::new(Vec::new()),
            history: Mutex::new(history),
        })
    }

    /// Create a new memory budget with a parent
    pub fn with_parent(config: BudgetConfig, parent: Arc<MemoryBudget>) -> Arc<Self> {
        let mut budget = Self::new(config);

        // Set parent and add self to parent's children
        let budget_mut = Arc::get_mut(&mut budget).unwrap();
        budget_mut.parent = Some(parent.clone());
        parent
            .children
            .lock()
            .unwrap()
            .push(Arc::downgrade(&budget));

        budget
    }

    /// Get the budget name
    pub fn name(&self) -> String {
        self.config.read().unwrap().name.clone()
    }

    /// Get the memory limit
    pub fn limit(&self) -> usize {
        self.config.read().unwrap().limit
    }

    /// Get the current memory usage
    pub fn used(&self) -> usize {
        *self.used.lock().unwrap()
    }

    /// Get the peak memory usage
    pub fn peak(&self) -> usize {
        *self.peak.lock().unwrap()
    }

    /// Get the parent budget (if any)
    pub fn parent(&self) -> Option<Arc<MemoryBudget>> {
        self.parent.clone()
    }

    /// Get the current budget state
    pub fn state(&self) -> BudgetState {
        let used = self.used();
        let config = self.config.read().unwrap();

        if config.limit == 0 {
            return BudgetState::Disabled;
        }

        let usage_percent = (used as f64 / config.limit as f64) * 100.0;

        if used > config.limit {
            BudgetState::Exceeded
        } else if usage_percent > 90.0 {
            BudgetState::Critical
        } else if usage_percent > 70.0 {
            BudgetState::HighUsage
        } else {
            BudgetState::Normal
        }
    }

    /// Get current metrics for the budget
    pub fn metrics(&self) -> BudgetMetrics {
        let used = self.used();
        let limit = self.limit();
        let peak = self.peak();
        let stats = self.stats.lock().unwrap();

        BudgetMetrics {
            used,
            limit,
            peak,
            allocations: stats.successful,
            failures: stats.failed,
            state: self.state(),
            timestamp: Instant::now(),
        }
    }

    /// Request memory allocation
    pub fn request_memory(&self, size: usize) -> MemoryResult<()> {
        if size == 0 {
            return Ok(());
        }

        let mut used = self.used.lock().unwrap();
        let config = self.config.read().unwrap();

        // Check if we can allocate
        let new_used = *used + size;
        let effective_limit = config.effective_limit();

        if new_used > effective_limit {
            let mut stats = self.stats.lock().unwrap();
            stats.failed += 1;

            return Err(MemoryError::allocation_failed());
        }

        // Update parent budget if needed
        if let Some(ref parent) = self.parent {
            parent.request_memory(size)?;
        }

        // Update usage statistics
        *used = new_used;

        let mut peak = self.peak.lock().unwrap();
        if new_used > *peak {
            *peak = new_used;
        }

        let mut stats = self.stats.lock().unwrap();
        stats.successful += 1;
        stats.total_bytes += size;
        if size > stats.largest_allocation {
            stats.largest_allocation = size;
        }

        // Update history if enabled
        if let Some(ref mut history) = *self.history.lock().unwrap() {
            history.add(new_used);
        }

        Ok(())
    }

    /// Release memory
    pub fn release_memory(&self, size: usize) {
        if size == 0 {
            return;
        }

        let mut used = self.used.lock().unwrap();

        // Ensure we don't underflow
        *used = used.saturating_sub(size);

        // Update history if enabled
        if let Some(ref mut history) = *self.history.lock().unwrap() {
            history.add(*used);
        }

        // Release from parent if needed
        if let Some(ref parent) = self.parent {
            parent.release_memory(size);
        }
    }

    /// Check if the budget can allocate the given amount
    pub fn can_allocate(&self, size: usize) -> bool {
        if size == 0 {
            return true;
        }

        let used = self.used();
        let config = self.config.read().unwrap();
        let effective_limit = config.effective_limit();

        // Check if we have enough memory
        if used + size > effective_limit {
            return false;
        }

        // Check parent if needed
        if let Some(ref parent) = self.parent {
            if !parent.can_allocate(size) {
                return false;
            }
        }

        true
    }

    /// Reset usage statistics
    pub fn reset_stats(&self) {
        *self.stats.lock().unwrap() = AllocationStats::default();
        *self.peak.lock().unwrap() = self.used();

        // Reset history if enabled
        if let Some(ref mut history) = *self.history.lock().unwrap() {
            history.timestamps.clear();
            history.usage.clear();
            history.last_update = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_allocation() {
        let budget = MemoryBudget::new(BudgetConfig::new("test", 1000));

        // Initial state
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.peak(), 0);
        assert_eq!(budget.state(), BudgetState::Normal);

        // Allocate memory
        assert!(budget.request_memory(500).is_ok());
        assert_eq!(budget.used(), 500);
        assert_eq!(budget.peak(), 500);

        // Allocate more memory
        assert!(budget.request_memory(300).is_ok());
        assert_eq!(budget.used(), 800);
        assert_eq!(budget.peak(), 800);
        assert_eq!(budget.state(), BudgetState::HighUsage);

        // Try to exceed limit
        assert!(budget.request_memory(300).is_err());
        assert_eq!(budget.used(), 800); // Unchanged

        // Release memory
        budget.release_memory(500);
        assert_eq!(budget.used(), 300);
        assert_eq!(budget.peak(), 800); // Peak remains unchanged
        assert_eq!(budget.state(), BudgetState::Normal);
    }

    #[test]
    fn test_budget_hierarchy() {
        let parent = MemoryBudget::new(BudgetConfig::new("parent", 1000));
        let child = MemoryBudget::with_parent(BudgetConfig::new("child", 500), parent.clone());

        // Initial state
        assert_eq!(parent.used(), 0);
        assert_eq!(child.used(), 0);

        // Allocate in child
        assert!(child.request_memory(300).is_ok());
        assert_eq!(child.used(), 300);
        assert_eq!(parent.used(), 300); // Parent usage is updated

        // Try to exceed child limit
        assert!(child.request_memory(300).is_err());

        // Try to exceed parent limit through child
        let child2 = MemoryBudget::with_parent(BudgetConfig::new("child2", 800), parent.clone());
        assert!(child2.request_memory(800).is_err()); // Would exceed parent's remaining capacity

        // Release from child
        child.release_memory(200);
        assert_eq!(child.used(), 100);
        assert_eq!(parent.used(), 100);
    }
}
