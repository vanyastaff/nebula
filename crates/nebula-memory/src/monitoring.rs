//! System-level memory monitoring and pressure handling
//!
//! This module integrates with nebula-system to provide real-time memory pressure
//! monitoring and automatic allocation strategy adjustment based on system state.

use core::time::Duration;

#[cfg(feature = "std")]
use std::sync::{Arc, Mutex};

use nebula_error::{ErrorKind, NebulaError, kinds::SystemError};
#[cfg(feature = "logging")]
use nebula_log::{debug, error, info, warn};
use nebula_system::memory::{self, MemoryInfo, MemoryPressure};

use crate::allocator::AllocatorStats;
use crate::core::config::MemoryConfig;
use crate::core::error::{MemoryError, MemoryResult};

/// System memory monitoring configuration
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    /// How often to check system memory pressure
    pub check_interval: Duration,
    /// Memory pressure threshold for warnings
    pub warning_threshold: MemoryPressure,
    /// Memory pressure threshold for emergency actions
    pub emergency_threshold: MemoryPressure,
    /// Enable automatic allocation strategy adjustment
    pub auto_adjust: bool,
    /// Maximum allocation size during high pressure
    pub high_pressure_max_alloc: usize,
    /// Enable detailed logging of memory events
    pub detailed_logging: bool,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(5),
            warning_threshold: MemoryPressure::High,
            emergency_threshold: MemoryPressure::Critical,
            auto_adjust: true,
            high_pressure_max_alloc: 64 * 1024, // 64KB
            detailed_logging: true,
        }
    }
}

/// Memory pressure action to take
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureAction {
    /// No action needed
    None,
    /// Log warning
    Warn,
    /// Reduce allocation sizes
    ReduceAllocations,
    /// Force garbage collection if possible
    ForceCleanup,
    /// Deny large allocations
    DenyLargeAllocations,
    /// Emergency: minimize all allocations
    Emergency,
}

/// System memory monitor
#[derive(Debug)]
pub struct MemoryMonitor {
    config: MonitoringConfig,
    #[cfg(feature = "std")]
    last_check: Arc<Mutex<Option<std::time::Instant>>>,
    last_pressure: MemoryPressure,
    pressure_change_count: usize,
}

impl MemoryMonitor {
    /// Create a new memory monitor with default configuration
    pub fn new() -> Self {
        Self::with_config(MonitoringConfig::default())
    }

    /// Create a new memory monitor with custom configuration
    pub fn with_config(config: MonitoringConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "std")]
            last_check: Arc::new(Mutex::new(None)),
            last_pressure: MemoryPressure::Low,
            pressure_change_count: 0,
        }
    }

    /// Check current memory pressure and get recommended action
    pub fn check_pressure(&mut self) -> MemoryResult<(MemoryInfo, PressureAction)> {
        let memory_info = memory::current();

        // Update pressure tracking
        if memory_info.pressure != self.last_pressure {
            self.pressure_change_count += 1;

            #[cfg(feature = "logging")]
            if self.config.detailed_logging {
                info!(
                    "Memory pressure changed: {:?} -> {:?} (change #{}) - {:.1}% used",
                    self.last_pressure,
                    memory_info.pressure,
                    self.pressure_change_count,
                    memory_info.usage_percent
                );
            }

            self.last_pressure = memory_info.pressure;
        }

        let action = self.determine_action(&memory_info);

        // Execute automatic actions if enabled
        if self.config.auto_adjust {
            self.execute_action(action, &memory_info)?;
        }

        Ok((memory_info, action))
    }

    /// Determine what action should be taken based on memory pressure
    fn determine_action(&self, memory_info: &MemoryInfo) -> PressureAction {
        match memory_info.pressure {
            MemoryPressure::Low => PressureAction::None,
            MemoryPressure::Medium => {
                if memory_info.pressure >= self.config.warning_threshold {
                    PressureAction::Warn
                } else {
                    PressureAction::None
                }
            }
            MemoryPressure::High => {
                if memory_info.pressure >= self.config.emergency_threshold {
                    PressureAction::Emergency
                } else {
                    PressureAction::ReduceAllocations
                }
            }
            MemoryPressure::Critical => PressureAction::Emergency,
        }
    }

    /// Execute the recommended action
    fn execute_action(&self, action: PressureAction, memory_info: &MemoryInfo) -> MemoryResult<()> {
        match action {
            PressureAction::None => {}
            PressureAction::Warn => {
                #[cfg(feature = "logging")]
                warn!(
                    "Memory pressure warning: {:.1}% used ({} / {})",
                    memory_info.usage_percent,
                    memory::format_bytes(memory_info.used),
                    memory::format_bytes(memory_info.total)
                );
            }
            PressureAction::ReduceAllocations => {
                #[cfg(feature = "logging")]
                warn!(
                    "High memory pressure: reducing allocation limits. Current: {:.1}% used",
                    memory_info.usage_percent
                );
            }
            PressureAction::ForceCleanup => {
                #[cfg(feature = "logging")]
                warn!(
                    "Forcing memory cleanup due to high pressure: {:.1}% used",
                    memory_info.usage_percent
                );
                // Note: actual cleanup would be implemented by the calling allocator
            }
            PressureAction::DenyLargeAllocations => {
                #[cfg(feature = "logging")]
                warn!(
                    "Denying large allocations due to memory pressure: {:.1}% used",
                    memory_info.usage_percent
                );
            }
            PressureAction::Emergency => {
                #[cfg(feature = "logging")]
                error!(
                    "EMERGENCY: Critical memory pressure at {:.1}% usage. Minimizing allocations.",
                    memory_info.usage_percent
                );
            }
        }
        Ok(())
    }

    /// Check if a large allocation should be allowed given current pressure
    pub fn should_allow_large_allocation(&mut self, size: usize) -> MemoryResult<bool> {
        let (memory_info, action) = self.check_pressure()?;

        let allowed = match action {
            PressureAction::None | PressureAction::Warn => true,
            PressureAction::ReduceAllocations => size <= self.config.high_pressure_max_alloc,
            PressureAction::ForceCleanup => size <= self.config.high_pressure_max_alloc / 2,
            PressureAction::DenyLargeAllocations | PressureAction::Emergency => {
                size <= self.config.high_pressure_max_alloc / 4
            }
        };

        #[cfg(feature = "logging")]
        if !allowed && self.config.detailed_logging {
            warn!(
                "Denying allocation of {} due to memory pressure ({:.1}% used, action: {:?})",
                memory::format_bytes(size),
                memory_info.usage_percent,
                action
            );
        }

        Ok(allowed)
    }

    /// Get current system memory information
    pub fn current_memory_info(&self) -> MemoryInfo {
        memory::current()
    }

    /// Check if we should trigger emergency cleanup
    pub fn should_emergency_cleanup(&mut self) -> MemoryResult<bool> {
        let (_, action) = self.check_pressure()?;
        Ok(action == PressureAction::Emergency)
    }

    /// Get monitoring statistics
    pub fn get_stats(&self) -> MonitoringStats {
        let memory_info = memory::current();
        MonitoringStats {
            current_pressure: memory_info.pressure,
            pressure_changes: self.pressure_change_count,
            memory_usage_percent: memory_info.usage_percent,
            total_memory: memory_info.total,
            available_memory: memory_info.available,
            used_memory: memory_info.used,
        }
    }

    /// Reset monitoring statistics
    pub fn reset_stats(&mut self) {
        self.pressure_change_count = 0;
        self.last_pressure = MemoryPressure::Low;
    }
}

impl Default for MemoryMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics from memory monitoring
#[derive(Debug, Clone)]
pub struct MonitoringStats {
    /// Current memory pressure level
    pub current_pressure: MemoryPressure,
    /// Number of pressure level changes observed
    pub pressure_changes: usize,
    /// Current memory usage percentage
    pub memory_usage_percent: f64,
    /// Total system memory in bytes
    pub total_memory: usize,
    /// Available system memory in bytes
    pub available_memory: usize,
    /// Used system memory in bytes
    pub used_memory: usize,
}

impl core::fmt::Display for MonitoringStats {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Memory Monitoring Statistics:")?;
        writeln!(f, "  Current pressure: {:?}", self.current_pressure)?;
        writeln!(f, "  Pressure changes: {}", self.pressure_changes)?;
        writeln!(f, "  Memory usage: {:.1}%", self.memory_usage_percent)?;
        writeln!(
            f,
            "  Total memory: {}",
            memory::format_bytes(self.total_memory)
        )?;
        writeln!(
            f,
            "  Available memory: {}",
            memory::format_bytes(self.available_memory)
        )?;
        writeln!(
            f,
            "  Used memory: {}",
            memory::format_bytes(self.used_memory)
        )?;
        Ok(())
    }
}

/// Integrated memory statistics combining allocator and system metrics
#[derive(Debug, Clone)]
pub struct IntegratedStats {
    /// Allocator-specific statistics
    pub allocator: AllocatorStats,
    /// System memory monitoring statistics
    pub monitoring: MonitoringStats,
    /// Combined metrics
    pub combined: CombinedMetrics,
}

/// Combined metrics from both allocator and system
#[derive(Debug, Clone)]
pub struct CombinedMetrics {
    /// Ratio of allocator usage to system memory
    pub allocator_to_system_ratio: f64,
    /// Estimated system impact of allocator
    pub estimated_system_impact: f64,
    /// Overall memory health score (0.0 = critical, 1.0 = excellent)
    pub health_score: f64,
}

impl IntegratedStats {
    /// Create integrated statistics from allocator and monitoring data
    pub fn new(allocator: AllocatorStats, monitoring: MonitoringStats) -> Self {
        let combined = Self::calculate_combined_metrics(&allocator, &monitoring);
        Self {
            allocator,
            monitoring,
            combined,
        }
    }

    /// Calculate combined metrics
    fn calculate_combined_metrics(
        allocator: &AllocatorStats,
        monitoring: &MonitoringStats,
    ) -> CombinedMetrics {
        let allocator_to_system_ratio = if monitoring.total_memory > 0 {
            allocator.allocated_bytes as f64 / monitoring.total_memory as f64
        } else {
            0.0
        };

        let estimated_system_impact = if monitoring.used_memory > 0 {
            allocator.allocated_bytes as f64 / monitoring.used_memory as f64
        } else {
            0.0
        };

        // Health score based on multiple factors
        let pressure_score = match monitoring.current_pressure {
            MemoryPressure::Low => 1.0,
            MemoryPressure::Medium => 0.7,
            MemoryPressure::High => 0.4,
            MemoryPressure::Critical => 0.1,
        };

        let efficiency_score = allocator.allocation_efficiency();
        let usage_score = 1.0 - (monitoring.memory_usage_percent / 100.0).min(1.0);

        let health_score = (pressure_score + efficiency_score + usage_score) / 3.0;

        CombinedMetrics {
            allocator_to_system_ratio,
            estimated_system_impact,
            health_score,
        }
    }
}

impl core::fmt::Display for IntegratedStats {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(f, "Integrated Memory Statistics:")?;
        writeln!(f)?;

        // Allocator stats
        write!(f, "{}", self.allocator)?;
        writeln!(f)?;

        // Monitoring stats
        write!(f, "{}", self.monitoring)?;
        writeln!(f)?;

        // Combined metrics
        writeln!(f, "Combined Metrics:")?;
        writeln!(
            f,
            "  Allocator to system ratio: {:.4}%",
            self.combined.allocator_to_system_ratio * 100.0
        )?;
        writeln!(
            f,
            "  Estimated system impact: {:.4}%",
            self.combined.estimated_system_impact * 100.0
        )?;
        writeln!(f, "  Health score: {:.2}/1.0", self.combined.health_score)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_monitor_creation() {
        let monitor = MemoryMonitor::new();
        assert_eq!(monitor.last_pressure, MemoryPressure::Low);
        assert_eq!(monitor.pressure_change_count, 0);
    }

    #[test]
    fn test_pressure_action_determination() {
        let monitor = MemoryMonitor::new();

        let low_pressure_info = MemoryInfo {
            total: 1000,
            available: 800,
            used: 200,
            usage_percent: 20.0,
            pressure: MemoryPressure::Low,
        };

        let action = monitor.determine_action(&low_pressure_info);
        assert_eq!(action, PressureAction::None);
    }

    #[test]
    fn test_integrated_stats() {
        let allocator_stats = AllocatorStats {
            allocated_bytes: 1024 * 1024, // 1MB
            allocation_count: 100,
            total_bytes_allocated: 2 * 1024 * 1024,
            ..Default::default()
        };

        let monitoring_stats = MonitoringStats {
            current_pressure: MemoryPressure::Low,
            pressure_changes: 5,
            memory_usage_percent: 30.0,
            total_memory: 8 * 1024 * 1024 * 1024,     // 8GB
            available_memory: 5 * 1024 * 1024 * 1024, // 5GB
            used_memory: 3 * 1024 * 1024 * 1024,      // 3GB
        };

        let integrated = IntegratedStats::new(allocator_stats, monitoring_stats);

        // Allocator uses 1MB out of 8GB total = ~0.000012% ratio
        assert!(integrated.combined.allocator_to_system_ratio < 0.001);

        // Health score should be relatively high with low pressure
        assert!(integrated.combined.health_score > 0.5);
    }
}
