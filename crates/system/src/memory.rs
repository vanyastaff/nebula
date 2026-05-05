//! Memory information and pressure detection
//!
//! Provides current memory usage snapshots and pressure classification
//! for backpressure and adaptive scaling decisions.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{
    availability::Availability,
    info::{MemoryCapacitySource, SystemInfo},
};

/// Memory pressure levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MemoryPressure {
    /// Memory pressure could not be classified from a valid sample
    Unavailable,
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
        matches!(
            self,
            MemoryPressure::High | MemoryPressure::Critical | MemoryPressure::Unavailable
        )
    }

    /// Check if memory pressure is critical
    #[must_use]
    pub fn is_critical(&self) -> bool {
        *self == MemoryPressure::Critical
    }
}

/// Reason a memory pressure level was assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MemoryPressureReason {
    /// Pressure is based on effective available-memory ratio.
    EffectiveAvailableRatio,
    /// Effective memory came from a Linux cgroup limit.
    CgroupLimit,
    /// Effective memory came from host memory.
    HostMemory,
    /// Total memory was unavailable or zero.
    MemoryUnavailable,
    /// Caller-supplied pressure thresholds failed validation.
    InvalidThresholds,
    /// Swap is disabled on the host or effective runtime.
    SwapDisabled,
    /// Swap is more than 80% used.
    SwapHigh,
}

/// Validation error for caller-supplied memory pressure thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MemoryPressureThresholdError {
    /// One or more threshold values was NaN or infinite.
    NonFinite,
    /// One or more threshold values was outside 0.0..=100.0.
    OutOfRange,
    /// Thresholds were not strictly ordered as medium < high < critical.
    NotStrictlyIncreasing,
}

impl std::fmt::Display for MemoryPressureThresholdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFinite => f.write_str("memory pressure thresholds must be finite"),
            Self::OutOfRange => {
                f.write_str("memory pressure thresholds must be within 0.0..=100.0")
            },
            Self::NotStrictlyIncreasing => {
                f.write_str("memory pressure thresholds must satisfy medium < high < critical")
            },
        }
    }
}

impl std::error::Error for MemoryPressureThresholdError {}

/// Thresholds used by the memory pressure classifier.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryPressureThresholds {
    /// Medium threshold in percent.
    pub medium_percent: f64,
    /// High threshold in percent.
    pub high_percent: f64,
    /// Critical threshold in percent.
    pub critical_percent: f64,
}

impl Default for MemoryPressureThresholds {
    fn default() -> Self {
        Self {
            medium_percent: 50.0,
            high_percent: 70.0,
            critical_percent: 85.0,
        }
    }
}

impl MemoryPressureThresholds {
    /// Validate threshold invariants before classification.
    ///
    /// Threshold values are percentages and must be finite, within
    /// `0.0..=100.0`, and strictly ordered as medium < high < critical.
    pub fn validate(&self) -> Result<(), MemoryPressureThresholdError> {
        let values = [
            self.medium_percent,
            self.high_percent,
            self.critical_percent,
        ];

        if values.iter().any(|value| !value.is_finite()) {
            return Err(MemoryPressureThresholdError::NonFinite);
        }

        if values.iter().any(|value| !(0.0..=100.0).contains(value)) {
            return Err(MemoryPressureThresholdError::OutOfRange);
        }

        if !(self.medium_percent < self.high_percent && self.high_percent < self.critical_percent) {
            return Err(MemoryPressureThresholdError::NotStrictlyIncreasing);
        }

        Ok(())
    }
}

/// Evidence used to classify memory pressure.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryPressureReport {
    /// Classified pressure level.
    pub level: MemoryPressure,
    /// Reasons that explain the level.
    pub reasons: Vec<MemoryPressureReason>,
    /// Effective total memory in bytes.
    pub effective_total: usize,
    /// Effective available memory in bytes.
    pub effective_available: usize,
    /// Effective used memory in bytes.
    pub effective_used: usize,
    /// Effective usage percentage.
    pub usage_percent: Availability<f64>,
    /// Host total memory in bytes.
    pub host_total: usize,
    /// Host available memory in bytes.
    pub host_available: usize,
    /// Total swap in bytes.
    pub swap_total: usize,
    /// Available swap in bytes.
    pub swap_available: usize,
    /// Effective capacity source.
    pub capacity_source: MemoryCapacitySource,
    /// Thresholds used by this report.
    pub thresholds: MemoryPressureThresholds,
}

/// Memory information
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryInfo {
    /// Effective total memory in bytes
    pub total: usize,
    /// Effective available memory in bytes
    pub available: usize,
    /// Effective used memory in bytes
    pub used: usize,
    /// Memory usage percentage
    pub usage_percent: Availability<f64>,
    /// Current memory pressure
    pub pressure: MemoryPressure,
    /// Host total physical memory in bytes
    pub host_total: usize,
    /// Host available physical memory in bytes
    pub host_available: usize,
    /// Total swap in bytes
    pub swap_total: usize,
    /// Available swap in bytes
    pub swap_available: usize,
    /// Effective capacity source used for scheduling-facing fields.
    pub capacity_source: MemoryCapacitySource,
    /// Evidence backing the pressure classification.
    pub pressure_report: MemoryPressureReport,
}

/// Get current memory information
#[must_use]
pub fn current() -> MemoryInfo {
    current_with_thresholds(MemoryPressureThresholds::default())
}

/// Get current memory information with caller-supplied pressure thresholds.
///
/// Invalid thresholds yield `MemoryPressure::Unavailable` with
/// `MemoryPressureReason::InvalidThresholds` in the pressure report.
#[must_use]
pub fn current_with_thresholds(thresholds: MemoryPressureThresholds) -> MemoryInfo {
    let sys_memory = SystemInfo::current_memory();
    let report = classify_memory_with_thresholds(&sys_memory, thresholds);

    MemoryInfo {
        total: report.effective_total,
        available: report.effective_available,
        used: report.effective_used,
        usage_percent: report.usage_percent.clone(),
        pressure: report.level,
        host_total: report.host_total,
        host_available: report.host_available,
        swap_total: report.swap_total,
        swap_available: report.swap_available,
        capacity_source: report.capacity_source,
        pressure_report: report,
    }
}

fn classify_memory(sys_memory: &crate::info::MemoryInfo) -> MemoryPressureReport {
    classify_memory_with_thresholds(sys_memory, MemoryPressureThresholds::default())
}

fn classify_memory_with_thresholds(
    sys_memory: &crate::info::MemoryInfo,
    thresholds: MemoryPressureThresholds,
) -> MemoryPressureReport {
    let total = sys_memory.effective.total;
    let available = sys_memory.effective.available;
    let used = total.saturating_sub(available);
    let mut reasons = Vec::new();

    match sys_memory.effective.source {
        MemoryCapacitySource::Host => reasons.push(MemoryPressureReason::HostMemory),
        MemoryCapacitySource::Cgroup => reasons.push(MemoryPressureReason::CgroupLimit),
    }

    if sys_memory.swap_total == 0 {
        reasons.push(MemoryPressureReason::SwapDisabled);
    } else {
        let swap_used = sys_memory
            .swap_total
            .saturating_sub(sys_memory.swap_available);
        if swap_used.saturating_mul(100) / sys_memory.swap_total > 80 {
            reasons.push(MemoryPressureReason::SwapHigh);
        }
    }

    let usage_percent = calculate_usage_percent(used, total);
    if thresholds.validate().is_err() {
        reasons.push(MemoryPressureReason::InvalidThresholds);
        if !usage_percent.is_available() {
            reasons.push(MemoryPressureReason::MemoryUnavailable);
        }

        return MemoryPressureReport {
            level: MemoryPressure::Unavailable,
            reasons,
            effective_total: total,
            effective_available: available,
            effective_used: used,
            usage_percent,
            host_total: sys_memory.total,
            host_available: sys_memory.available,
            swap_total: sys_memory.swap_total,
            swap_available: sys_memory.swap_available,
            capacity_source: sys_memory.effective.source,
            thresholds,
        };
    }

    let level = match usage_percent.value().copied() {
        _ if total == 0 => {
            reasons.push(MemoryPressureReason::MemoryUnavailable);
            MemoryPressure::Unavailable
        },
        Some(percent) if percent > thresholds.critical_percent => MemoryPressure::Critical,
        Some(percent) if percent > thresholds.high_percent => MemoryPressure::High,
        Some(percent) if percent > thresholds.medium_percent => MemoryPressure::Medium,
        Some(_) => MemoryPressure::Low,
        None => {
            reasons.push(MemoryPressureReason::MemoryUnavailable);
            MemoryPressure::Unavailable
        },
    };

    if level != MemoryPressure::Unavailable {
        reasons.push(MemoryPressureReason::EffectiveAvailableRatio);
    }

    MemoryPressureReport {
        level,
        reasons,
        effective_total: total,
        effective_available: available,
        effective_used: used,
        usage_percent,
        host_total: sys_memory.total,
        host_available: sys_memory.available,
        swap_total: sys_memory.swap_total,
        swap_available: sys_memory.swap_available,
        capacity_source: sys_memory.effective.source,
        thresholds,
    }
}

fn calculate_usage_percent(used: usize, total: usize) -> Availability<f64> {
    // Calculate usage percent with checked arithmetic to avoid precision loss.
    // For very large memory values, direct f64 conversion can lose precision.
    // We use checked_mul to compute (used * 10000) / total, then divide by 100
    // to get percentage with 2 decimal precision.
    if total == 0 {
        return Availability::unavailable("total memory is zero or unavailable");
    }

    used.checked_mul(10000)
        .and_then(|v| v.checked_div(total))
        .map_or_else(
            || Availability::available((used as f64 / total as f64) * 100.0),
            |v| Availability::available(v as f64 / 100.0),
        )
}

/// Get current memory pressure
#[must_use]
pub fn pressure() -> MemoryPressure {
    current().pressure
}

/// Get current memory pressure with raw evidence.
#[must_use]
pub fn pressure_report() -> MemoryPressureReport {
    classify_memory(&SystemInfo::current_memory())
}

/// Get current memory pressure with caller-supplied thresholds.
///
/// Invalid thresholds yield `MemoryPressure::Unavailable` with
/// `MemoryPressureReason::InvalidThresholds` in the report.
#[must_use]
pub fn pressure_report_with_thresholds(
    thresholds: MemoryPressureThresholds,
) -> MemoryPressureReport {
    classify_memory_with_thresholds(&SystemInfo::current_memory(), thresholds)
}

/// Format bytes for human-readable display
///
/// Re-exported from utils for convenience.
pub use crate::utils::format_bytes_usize as format_bytes;

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::{
        MemoryPressure, MemoryPressureReason, MemoryPressureThresholdError,
        MemoryPressureThresholds, classify_memory, classify_memory_with_thresholds,
    };
    use crate::{
        Availability,
        info::{
            CgroupMemoryInfo, EffectiveMemoryInfo, MemoryCapacitySource,
            MemoryInfo as SystemMemoryInfo, SnapshotFreshness, SnapshotMetadata,
        },
    };

    fn fresh_test_metadata() -> SnapshotMetadata {
        SnapshotMetadata {
            observed_at: SystemTime::UNIX_EPOCH,
            freshness: SnapshotFreshness::Fresh,
            source: "test".to_string(),
        }
    }

    fn system_memory(total: usize, available: usize) -> SystemMemoryInfo {
        SystemMemoryInfo {
            metadata: fresh_test_metadata(),
            total,
            available,
            page_size: 4096,
            swap_total: 0,
            swap_available: 0,
            effective: EffectiveMemoryInfo {
                total,
                available,
                source: MemoryCapacitySource::Host,
            },
            cgroup: Availability::unavailable("not detected"),
        }
    }

    fn system_memory_with_cgroup(total: usize, available: usize) -> SystemMemoryInfo {
        SystemMemoryInfo {
            metadata: fresh_test_metadata(),
            total: 64 * 1024,
            available: 48 * 1024,
            page_size: 4096,
            swap_total: 0,
            swap_available: 0,
            effective: EffectiveMemoryInfo {
                total,
                available,
                source: MemoryCapacitySource::Cgroup,
            },
            cgroup: Availability::available(CgroupMemoryInfo {
                total,
                free: available,
                free_swap: 0,
                rss: total.saturating_sub(available),
            }),
        }
    }

    #[test]
    fn pressure_boundaries_use_strict_greater_than_thresholds() {
        let total = 1000;

        assert_eq!(
            classify_memory(&system_memory(total, 500)).level,
            MemoryPressure::Low
        );
        assert_eq!(
            classify_memory(&system_memory(total, 499)).level,
            MemoryPressure::Medium
        );
        assert_eq!(
            classify_memory(&system_memory(total, 300)).level,
            MemoryPressure::Medium
        );
        assert_eq!(
            classify_memory(&system_memory(total, 299)).level,
            MemoryPressure::High
        );
        assert_eq!(
            classify_memory(&system_memory(total, 150)).level,
            MemoryPressure::High
        );
        assert_eq!(
            classify_memory(&system_memory(total, 149)).level,
            MemoryPressure::Critical
        );
    }

    #[test]
    fn zero_total_memory_is_unavailable_with_no_usage_percent() {
        let report = classify_memory(&system_memory(0, 0));
        assert_eq!(report.level, MemoryPressure::Unavailable);
        assert!(!report.usage_percent.is_available());
        assert!(
            report
                .reasons
                .contains(&MemoryPressureReason::MemoryUnavailable)
        );
    }

    #[test]
    fn cgroup_capacity_source_is_reported_as_evidence() {
        let report = classify_memory(&system_memory_with_cgroup(2048, 512));
        assert_eq!(report.capacity_source, MemoryCapacitySource::Cgroup);
        assert_eq!(report.effective_total, 2048);
        assert_eq!(report.effective_available, 512);
        assert!(report.reasons.contains(&MemoryPressureReason::CgroupLimit));
    }

    #[test]
    fn caller_supplied_thresholds_change_pressure_classification() {
        let thresholds = MemoryPressureThresholds {
            medium_percent: 20.0,
            high_percent: 40.0,
            critical_percent: 60.0,
        };

        let report = classify_memory_with_thresholds(&system_memory(1000, 500), thresholds);

        assert_eq!(report.level, MemoryPressure::High);
        assert_eq!(report.thresholds, thresholds);
    }

    #[test]
    fn default_thresholds_validate() {
        assert_eq!(MemoryPressureThresholds::default().validate(), Ok(()));
    }

    #[test]
    fn invalid_thresholds_make_pressure_unavailable() {
        let invalid_thresholds = [
            (
                MemoryPressureThresholds {
                    medium_percent: f64::NAN,
                    high_percent: 70.0,
                    critical_percent: 85.0,
                },
                MemoryPressureThresholdError::NonFinite,
            ),
            (
                MemoryPressureThresholds {
                    medium_percent: -1.0,
                    high_percent: 70.0,
                    critical_percent: 85.0,
                },
                MemoryPressureThresholdError::OutOfRange,
            ),
            (
                MemoryPressureThresholds {
                    medium_percent: 70.0,
                    high_percent: 70.0,
                    critical_percent: 85.0,
                },
                MemoryPressureThresholdError::NotStrictlyIncreasing,
            ),
        ];

        for (thresholds, expected_error) in invalid_thresholds {
            assert_eq!(thresholds.validate(), Err(expected_error));

            let report = classify_memory_with_thresholds(&system_memory(1000, 0), thresholds);

            assert_eq!(report.level, MemoryPressure::Unavailable);
            assert_eq!(report.usage_percent.value().copied(), Some(100.0));
            assert!(
                report
                    .reasons
                    .contains(&MemoryPressureReason::InvalidThresholds)
            );
        }
    }

    #[test]
    fn swap_pressure_is_reported_as_a_reason() {
        let mut memory = system_memory(1000, 600);
        memory.swap_total = 1000;
        memory.swap_available = 100;

        let report = classify_memory(&memory);
        assert!(report.reasons.contains(&MemoryPressureReason::SwapHigh));
        assert!(!report.reasons.contains(&MemoryPressureReason::SwapDisabled));
    }
}
