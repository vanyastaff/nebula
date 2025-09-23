//! Statistics and monitoring for memory management
//!
//! This module provides comprehensive memory statistics collection,
//! tracking, and analysis capabilities with minimal overhead.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

// Core statistics types
pub mod config;
pub mod counter;
pub mod memory_stats;

// Historical tracking
#[cfg(feature = "stats")]
pub mod tracker;

// Real-time monitoring
#[cfg(all(feature = "stats", feature = "std"))]
pub mod real_time;

// Aggregation and analysis
#[cfg(all(feature = "stats", feature = "std"))]
pub mod aggregator;

// Snapshots and comparisons
#[cfg(all(feature = "stats", feature = "std"))]
pub mod snapshot;

// Histogram analysis
#[cfg(feature = "stats")]
pub mod histogram;

// Predictive analytics
#[cfg(all(feature = "stats", feature = "std"))]
pub mod predictive;

// Detailed profiling
#[cfg(all(feature = "profiling", feature = "std"))]
pub mod profiler;

// Re-exports for convenience
#[cfg(all(feature = "stats", feature = "std"))]
pub use aggregator::{AggregatedStats, Aggregator, HistoricalMetricsSummary};
pub use config::{
    AlertConfig, HistogramConfig, MonitoringConfig, PerformanceImpact, StatsConfig, TrackedMetric,
    TrackingConfig, TrackingLevel,
};
pub use counter::{Counter, CounterType};
#[cfg(feature = "stats")]
pub use histogram::{HistogramData, MemoryHistogram, Percentile};
pub use memory_stats::{MemoryMetrics, MemoryStats};
#[cfg(all(feature = "stats", feature = "std"))]
pub use predictive::{MemoryTrend, Prediction, PredictionModel, PredictiveAnalytics, TrendType};
#[cfg(all(feature = "profiling", feature = "std"))]
pub use profiler::{AllocationSite, HotSpot, MemoryProfiler, ProfileReport};
#[cfg(all(feature = "stats", feature = "std"))]
pub use real_time::{MemoryAlert, RealTimeData, RealTimeMonitor};
#[cfg(all(feature = "stats", feature = "std"))]
pub use snapshot::{MemorySnapshot, SnapshotDiff, SnapshotFormat};
#[cfg(feature = "stats")]
pub use tracker::{DataPoint, MemoryTracker, WindowStats};

/// Initialize global statistics system
pub fn initialize(config: StatsConfig) -> crate::error::MemoryResult<()> {
    config
        .validate()
        .map_err(|e| crate::error::MemoryError::InvalidConfig { reason: e.to_string() })?;
    // Initialize any global state if needed
    Ok(())
}

/// Performance utilities
pub mod utils {
    pub use crate::utils::{format_bytes, format_duration, format_percentage, format_rate};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_accessible() {
        let _stats = MemoryStats::default();
    }
}
