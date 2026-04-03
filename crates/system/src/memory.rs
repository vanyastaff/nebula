//! Memory information and pressure detection
//!
//! Provides current memory usage snapshots and pressure classification
//! for backpressure and adaptive scaling decisions.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::info::SystemInfo;

/// Memory pressure levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MemoryPressure {
    /// Less than 50% memory used
    Low,
    /// 50-70% memory used
    Medium,
    /// 70-85% memory used
    High,
    /// More than 85% memory used
    Critical,
}

impl MemoryPressure {
    /// Check if memory pressure is concerning (High or Critical)
    #[must_use]
    pub fn is_concerning(&self) -> bool {
        *self >= MemoryPressure::High
    }

    /// Check if memory pressure is critical
    #[must_use]
    pub fn is_critical(&self) -> bool {
        *self == MemoryPressure::Critical
    }
}

/// Memory information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryInfo {
    /// Total physical memory in bytes
    pub total: usize,
    /// Available physical memory in bytes
    pub available: usize,
    /// Used physical memory in bytes
    pub used: usize,
    /// Memory usage percentage
    pub usage_percent: f64,
    /// Current memory pressure
    pub pressure: MemoryPressure,
}

/// Get current memory information
#[must_use]
pub fn current() -> MemoryInfo {
    let sys_memory = SystemInfo::current_memory();
    let used = sys_memory.total.saturating_sub(sys_memory.available);

    // Calculate usage percent with checked arithmetic to avoid precision loss
    // For very large memory values, direct f64 conversion can lose precision.
    // We use checked_mul to compute (used * 10000) / total, then divide by 100
    // to get percentage with 2 decimal precision.
    let usage_percent = if sys_memory.total > 0 {
        used.checked_mul(10000)
            .and_then(|v| v.checked_div(sys_memory.total))
            .map_or_else(
                || {
                    // Fallback to direct f64 if overflow (extremely rare)
                    (used as f64 / sys_memory.total as f64) * 100.0
                },
                |v| v as f64 / 100.0,
            )
    } else {
        0.0
    };

    let pressure = if usage_percent > 85.0 {
        MemoryPressure::Critical
    } else if usage_percent > 70.0 {
        MemoryPressure::High
    } else if usage_percent > 50.0 {
        MemoryPressure::Medium
    } else {
        MemoryPressure::Low
    };

    MemoryInfo {
        total: sys_memory.total,
        available: sys_memory.available,
        used,
        usage_percent,
        pressure,
    }
}

/// Get current memory pressure
#[must_use]
pub fn pressure() -> MemoryPressure {
    current().pressure
}

/// Format bytes for human-readable display
///
/// Re-exported from utils for convenience.
pub use crate::utils::format_bytes_usize as format_bytes;
