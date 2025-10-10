//! Configuration for the memory budgeting system
//!
//! This module defines the configuration options for the memory budgeting system,
//! including policies for overcommitment, reservation modes, and other settings.

use std::fmt;
use std::time::Duration;

/// Policy for handling memory overcommitment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OvercommitPolicy {
    /// Never allow overcommitment
    None,

    /// Allow overcommitment up to a percentage of the budget
    Percentage(u8),

    /// Allow overcommitment up to a fixed amount
    Fixed(usize),

    /// Allow unlimited overcommitment (dangerous)
    Unlimited,
}

impl Default for OvercommitPolicy {
    fn default() -> Self {
        Self::Percentage(10) // Default to 10% overcommitment
    }
}

/// Mode for memory reservations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationMode {
    /// Strict reservations that guarantee memory availability
    Strict,

    /// Best-effort reservations that may fail under pressure
    BestEffort,

    /// Elastic reservations that can shrink under pressure
    Elastic,
}

impl Default for ReservationMode {
    fn default() -> Self {
        Self::BestEffort
    }
}

/// Configuration for the memory budgeting system
#[derive(Debug, Clone)]
pub struct BudgetConfig {
    /// Name of the budget
    pub name: String,

    /// Memory limit in bytes
    pub limit: usize,

    /// Overcommit policy
    pub overcommit_policy: OvercommitPolicy,

    /// Reservation mode
    pub reservation_mode: ReservationMode,

    /// Minimum guaranteed memory (bytes)
    pub min_guaranteed: usize,

    /// Time window for usage tracking (None for unlimited)
    pub tracking_window: Option<Duration>,

    /// Enable adaptive behavior based on system pressure
    pub adaptive: bool,

    /// Priority level (higher = more important)
    pub priority: u8,

    /// Enable detailed statistics collection
    pub collect_stats: bool,
}

impl BudgetConfig {
    /// Create a new budget configuration with the given name and limit
    pub fn new(name: impl Into<String>, limit: usize) -> Self {
        Self {
            name: name.into(),
            limit,
            overcommit_policy: OvercommitPolicy::default(),
            reservation_mode: ReservationMode::default(),
            min_guaranteed: 0,
            tracking_window: Some(Duration::from_secs(60)),
            adaptive: true,
            priority: 50,
            collect_stats: true,
        }
    }

    /// Set the overcommit policy
    #[must_use = "builder methods must be chained or built"]
    pub fn with_overcommit(mut self, policy: OvercommitPolicy) -> Self {
        self.overcommit_policy = policy;
        self
    }

    /// Set the reservation mode
    #[must_use = "builder methods must be chained or built"]
    pub fn with_reservation_mode(mut self, mode: ReservationMode) -> Self {
        self.reservation_mode = mode;
        self
    }

    /// Set the minimum guaranteed memory
    #[must_use = "builder methods must be chained or built"]
    pub fn with_min_guaranteed(mut self, min: usize) -> Self {
        self.min_guaranteed = min;
        self
    }

    /// Set the tracking window
    #[must_use = "builder methods must be chained or built"]
    pub fn with_tracking_window(mut self, window: Option<Duration>) -> Self {
        self.tracking_window = window;
        self
    }

    /// Enable or disable adaptive behavior
    #[must_use = "builder methods must be chained or built"]
    pub fn with_adaptive(mut self, adaptive: bool) -> Self {
        self.adaptive = adaptive;
        self
    }

    /// Set the priority level
    #[must_use = "builder methods must be chained or built"]
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Enable or disable statistics collection
    #[must_use = "builder methods must be chained or built"]
    pub fn with_stats(mut self, collect_stats: bool) -> Self {
        self.collect_stats = collect_stats;
        self
    }

    /// Calculate the effective limit including overcommitment
    pub fn effective_limit(&self) -> usize {
        match self.overcommit_policy {
            OvercommitPolicy::None => self.limit,
            OvercommitPolicy::Percentage(pct) => self.limit + (self.limit * pct as usize / 100),
            OvercommitPolicy::Fixed(amount) => self.limit + amount,
            OvercommitPolicy::Unlimited => usize::MAX,
        }
    }

    /// Validate the configuration
    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), String> {
        if self.limit == 0 {
            return Err("Memory limit cannot be zero".to_string());
        }

        if self.min_guaranteed > self.limit {
            return Err(format!(
                "Minimum guaranteed memory ({}) cannot exceed limit ({})",
                self.min_guaranteed, self.limit
            ));
        }

        Ok(())
    }
}

impl fmt::Display for BudgetConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BudgetConfig {{ name: {}, limit: {} bytes, priority: {} }}",
            self.name, self.limit, self.priority
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_config_defaults() {
        let config = BudgetConfig::new("test", 1024 * 1024);
        assert_eq!(config.name, "test");
        assert_eq!(config.limit, 1024 * 1024);
        assert_eq!(config.overcommit_policy, OvercommitPolicy::Percentage(10));
        assert_eq!(config.reservation_mode, ReservationMode::BestEffort);
        assert_eq!(config.min_guaranteed, 0);
        assert!(config.tracking_window.is_some());
        assert!(config.adaptive);
        assert_eq!(config.priority, 50);
        assert!(config.collect_stats);
    }

    #[test]
    fn test_effective_limit() {
        let config =
            BudgetConfig::new("test", 1000).with_overcommit(OvercommitPolicy::Percentage(20));
        assert_eq!(config.effective_limit(), 1200); // 1000 + 20%

        let config = BudgetConfig::new("test", 1000).with_overcommit(OvercommitPolicy::Fixed(500));
        assert_eq!(config.effective_limit(), 1500); // 1000 + 500

        let config = BudgetConfig::new("test", 1000).with_overcommit(OvercommitPolicy::None);
        assert_eq!(config.effective_limit(), 1000); // No overcommit
    }

    #[test]
    fn test_validation() {
        let config = BudgetConfig::new("test", 0);
        assert!(config.validate().is_err());

        let config = BudgetConfig::new("test", 1000).with_min_guaranteed(2000);
        assert!(config.validate().is_err());

        let config = BudgetConfig::new("test", 1000).with_min_guaranteed(500);
        assert!(config.validate().is_ok());
    }
}
