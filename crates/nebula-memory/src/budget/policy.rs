//! Allocation policies for the memory budgeting system
//!
//! This module provides various allocation policies that determine how memory
//! is allocated and managed across different budgets and contexts.

use std::cmp::Ordering;
use std::sync::Arc;

use super::budget::{MemoryBudget, BudgetState};
use super::config::BudgetConfig;

/// Interface for memory allocation policies
pub trait AllocationPolicy: Send + Sync {
    /// Determine if a memory allocation should be allowed
    fn allow_allocation(
        &self,
        budget: &MemoryBudget,
        size: usize,
        context: Option<&str>,
    ) -> AllocationDecision;
    
    /// Suggest memory to reclaim when under pressure
    fn suggest_reclaim(
        &self,
        budgets: &[Arc<MemoryBudget>],
        target_size: usize,
    ) -> Vec<ReclaimSuggestion>;
    
    /// Handle memory pressure situation
    fn handle_pressure(&self, budgets: &[Arc<MemoryBudget>], severity: PressureSeverity);
}

/// Decision for a memory allocation request
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocationDecision {
    /// Allow the allocation
    Allow,
    
    /// Deny the allocation
    Deny { reason: String },
    
    /// Allow the allocation, but with a warning
    AllowWithWarning { message: String },
    
    /// Defer the decision to another policy
    Defer,
}

/// Suggestion for memory reclamation
#[derive(Debug, Clone)]
pub struct ReclaimSuggestion {
    /// Budget to reclaim memory from
    pub budget: Arc<MemoryBudget>,
    
    /// Amount of memory to reclaim
    pub amount: usize,
    
    /// Priority of the suggestion (higher = more important)
    pub priority: u8,
}

/// Severity of memory pressure
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PressureSeverity {
    /// Low memory pressure (>70% usage)
    Low = 0,
    
    /// Medium memory pressure (>85% usage)
    Medium = 1,
    
    /// High memory pressure (>95% usage)
    High = 2,
    
    /// Critical memory pressure (>99% usage)
    Critical = 3,
}

/// Policy that allocates based on priority
pub struct PriorityPolicy {
    /// Minimum priority to allow allocation under pressure
    min_priority_under_pressure: u8,
    
    /// Whether to allow overcommitment for high-priority allocations
    allow_overcommit_for_high_priority: bool,
}

impl PriorityPolicy {
    /// Create a new priority policy
    pub fn new(min_priority_under_pressure: u8, allow_overcommit_for_high_priority: bool) -> Self {
        Self {
            min_priority_under_pressure,
            allow_overcommit_for_high_priority,
        }
    }
}

impl Default for PriorityPolicy {
    fn default() -> Self {
        Self {
            min_priority_under_pressure: 80, // Higher priority = more important (0-100)
            allow_overcommit_for_high_priority: true,
        }
    }
}

impl AllocationPolicy for PriorityPolicy {
    fn allow_allocation(
        &self,
        budget: &MemoryBudget,
        size: usize,
        context: Option<&str>,
    ) -> AllocationDecision {
        // Check if budget can allocate normally
        if budget.can_allocate(size) {
            return AllocationDecision::Allow;
        }
        
        // If we're here, the budget can't allocate normally
        let config = budget.config.read().unwrap();
        
        // Check if we're under pressure
        let is_under_pressure = matches!(budget.state(), BudgetState::Critical | BudgetState::Exceeded);
        
        if is_under_pressure {
            // Under pressure, only allow high-priority allocations
            if config.priority >= self.min_priority_under_pressure {
                if self.allow_overcommit_for_high_priority {
                    AllocationDecision::AllowWithWarning {
                        message: format!(
                            "Allowing high-priority allocation of {} bytes despite pressure",
                            size
                        ),
                    }
                } else {
                    AllocationDecision::Deny {
                        reason: format!(
                            "Budget under pressure, allocation of {} bytes denied despite high priority",
                            size
                        ),
                    }
                }
            } else {
                AllocationDecision::Deny {
                    reason: format!(
                        "Budget under pressure, allocation of {} bytes denied due to low priority",
                        size
                    ),
                }
            }
        } else {
            // Not under pressure, but still can't allocate
            AllocationDecision::Deny {
                reason: format!("Budget cannot allocate {} bytes", size),
            }
        }
    }
    
    fn suggest_reclaim(
        &self,
        budgets: &[Arc<MemoryBudget>],
        target_size: usize,
    ) -> Vec<ReclaimSuggestion> {
        // Sort budgets by priority (lowest priority first)
        let mut sorted_budgets = budgets.to_vec();
        sorted_budgets.sort_by(|a, b| {
            let a_priority = a.config.read().unwrap().priority;
            let b_priority = b.config.read().unwrap().priority;
            a_priority.cmp(&b_priority)
        });
        
        let mut suggestions = Vec::new();
        let mut total_suggested = 0;
        
        // Suggest reclaiming from lowest priority budgets first
        for budget in sorted_budgets {
            let used = budget.used();
            let config = budget.config.read().unwrap();
            
            // Skip empty budgets
            if used == 0 {
                continue;
            }
            
            // Calculate how much to reclaim from this budget
            let reclaim_amount = if total_suggested < target_size {
                // Still need more memory
                let needed = target_size - total_suggested;
                let available = used.saturating_sub(config.min_guaranteed);
                available.min(needed)
            } else {
                // Already have enough suggestions
                0
            };
            
            if reclaim_amount > 0 {
                suggestions.push(ReclaimSuggestion {
                    budget: budget.clone(),
                    amount: reclaim_amount,
                    priority: 100 - config.priority, // Invert priority for suggestion priority
                });
                
                total_suggested += reclaim_amount;
                
                if total_suggested >= target_size {
                    break;
                }
            }
        }
        
        suggestions
    }
    
    fn handle_pressure(&self, budgets: &[Arc<MemoryBudget>], severity: PressureSeverity) {
        // In a real implementation, this would take actions to reduce memory pressure
        // For now, we just log the pressure
        match severity {
            PressureSeverity::Low => {
                // Maybe log a warning
            }
            PressureSeverity::Medium => {
                // Start reclaiming from low-priority budgets
                let target = 1024 * 1024; // 1MB
                let suggestions = self.suggest_reclaim(budgets, target);
                
                for suggestion in suggestions {
                    // In a real implementation, we would actually reclaim memory
                    // For now, we just log the suggestion
                    let _ = suggestion;
                }
            }
            PressureSeverity::High | PressureSeverity::Critical => {
                // Aggressively reclaim from all but the highest priority budgets
                let target = 10 * 1024 * 1024; // 10MB
                let suggestions = self.suggest_reclaim(budgets, target);
                
                for suggestion in suggestions {
                    // In a real implementation, we would actually reclaim memory
                    // For now, we just log the suggestion
                    let _ = suggestion;
                }
            }
        }
    }
}

/// Policy that allocates based on fair sharing
pub struct FairSharePolicy {
    /// Minimum share percentage for each budget
    min_share_percent: u8,
}

impl FairSharePolicy {
    /// Create a new fair share policy
    pub fn new(min_share_percent: u8) -> Self {
        Self { min_share_percent }
    }
}

impl Default for FairSharePolicy {
    fn default() -> Self {
        Self { min_share_percent: 5 } // Minimum 5% share for each budget
    }
}

impl AllocationPolicy for FairSharePolicy {
    fn allow_allocation(
        &self,
        budget: &MemoryBudget,
        size: usize,
        context: Option<&str>,
    ) -> AllocationDecision {
        // Check if budget can allocate normally
        if budget.can_allocate(size) {
            return AllocationDecision::Allow;
        }
        
        // If we're here, the budget can't allocate normally
        // In a fair share policy, we would check if the budget has used its fair share
        
        // For now, just deny the allocation
        AllocationDecision::Deny {
            reason: format!("Budget cannot allocate {} bytes", size),
        }
    }
    
    fn suggest_reclaim(
        &self,
        budgets: &[Arc<MemoryBudget>],
        target_size: usize,
    ) -> Vec<ReclaimSuggestion> {
        // Calculate fair share for each budget
        let total_limit: usize = budgets.iter().map(|b| b.limit()).sum();
        let budget_count = budgets.len();
        
        if total_limit == 0 || budget_count == 0 {
            return Vec::new();
        }
        
        let fair_share_percent = 100 / budget_count as u8;
        let min_share_percent = self.min_share_percent.min(fair_share_percent);
        
        let mut suggestions = Vec::new();
        let mut total_suggested = 0;
        
        // Find budgets that are using more than their fair share
        for budget in budgets {
            let used = budget.used();
            let limit = budget.limit();
            
            // Calculate fair share for this budget
            let fair_share = (total_limit * fair_share_percent as usize) / 100;
            let min_share = (total_limit * min_share_percent as usize) / 100;
            
            // If budget is using more than its fair share, suggest reclaiming the excess
            if used > fair_share {
                let excess = used - fair_share;
                let reclaim_amount = if total_suggested < target_size {
                    // Still need more memory
                    let needed = target_size - total_suggested;
                    let available = excess.min(used.saturating_sub(min_share));
                    available.min(needed)
                } else {
                    // Already have enough suggestions
                    0
                };
                
                if reclaim_amount > 0 {
                    suggestions.push(ReclaimSuggestion {
                        budget: budget.clone(),
                        amount: reclaim_amount,
                        priority: ((used as f64 / limit as f64) * 100.0) as u8, // Higher usage % = higher priority
                    });
                    
                    total_suggested += reclaim_amount;
                    
                    if total_suggested >= target_size {
                        break;
                    }
                }
            }
        }
        
        suggestions
    }
    
    fn handle_pressure(&self, budgets: &[Arc<MemoryBudget>], severity: PressureSeverity) {
        // Similar to PriorityPolicy, but using fair share calculations
        match severity {
            PressureSeverity::Low => {
                // Maybe log a warning
            }
            PressureSeverity::Medium => {
                // Start reclaiming from budgets exceeding fair share
                let target = 1024 * 1024; // 1MB
                let suggestions = self.suggest_reclaim(budgets, target);
                
                for suggestion in suggestions {
                    // In a real implementation, we would actually reclaim memory
                    // For now, we just log the suggestion
                    let _ = suggestion;
                }
            }
            PressureSeverity::High | PressureSeverity::Critical => {
                // Aggressively reclaim from all budgets exceeding fair share
                let target = 10 * 1024 * 1024; // 10MB
                let suggestions = self.suggest_reclaim(budgets, target);
                
                for suggestion in suggestions {
                    // In a real implementation, we would actually reclaim memory
                    // For now, we just log the suggestion
                    let _ = suggestion;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_priority_policy() {
        let policy = PriorityPolicy::default();
        let config = BudgetConfig::new("test", 1000)
            .with_priority(50); // Medium priority
        let budget = MemoryBudget::new(config);
        
        // Allocate up to the limit
        assert!(budget.request_memory(900).is_ok());
        
        // Check policy decision for allocation within limit
        let decision = policy.allow_allocation(&budget, 50, None);
        assert!(matches!(decision, AllocationDecision::Allow));
        
        // Check policy decision for allocation exceeding limit
        let decision = policy.allow_allocation(&budget, 200, None);
        assert!(matches!(decision, AllocationDecision::Deny { .. }));
        
        // Create a high-priority budget
        let high_config = BudgetConfig::new("high", 1000)
            .with_priority(90); // High priority
        let high_budget = MemoryBudget::new(high_config);
        
        // Allocate up to the limit
        assert!(high_budget.request_memory(900).is_ok());
        
        // Check policy decision for high-priority allocation exceeding limit
        let decision = policy.allow_allocation(&high_budget, 200, None);
        assert!(matches!(decision, AllocationDecision::AllowWithWarning { .. }));
    }
    
    #[test]
    fn test_fair_share_policy() {
        let policy = FairSharePolicy::default();
        
        // Create two budgets with equal limits
        let budget1 = MemoryBudget::new(BudgetConfig::new("budget1", 1000));
        let budget2 = MemoryBudget::new(BudgetConfig::new("budget2", 1000));
        
        // Allocate different amounts
        assert!(budget1.request_memory(800).is_ok()); // 80% of limit
        assert!(budget2.request_memory(200).is_ok()); // 20% of limit
        
        // Check reclaim suggestions
        let suggestions = policy.suggest_reclaim(
            &[budget1.clone(), budget2.clone()],
            500,
        );
        
        // Should suggest reclaiming from budget1 since it's using more than fair share
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].budget.name(), "budget1");
        assert!(suggestions[0].amount > 0);
    }
}