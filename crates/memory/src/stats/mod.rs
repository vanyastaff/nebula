//!
//! Statistics and monitoring for memory management
//! This module provides comprehensive memory statistics collection,
//! tracking, and analysis capabilities with minimal overhead.

// Core statistics types
pub mod config;
pub mod counter;
pub mod memory_stats;

// Collection and global stats (always available)
pub mod collector;

// Export formats
#[cfg(feature = "stats")]
pub mod export;

// Historical tracking
#[cfg(feature = "stats")]
pub mod tracker;

// Real-time monitoring
#[cfg(feature = "stats")]
pub mod real_time;

// Aggregation and analysis
#[cfg(feature = "stats")]
pub mod aggregator;

// Snapshots and comparisons
#[cfg(feature = "stats")]
pub mod snapshot;

// Histogram analysis
#[cfg(feature = "stats")]
pub mod histogram;

// Predictive analytics
#[cfg(feature = "stats")]
pub mod predictive;

// Detailed profiling
#[cfg(feature = "profiling")]
pub mod profiler;

// Re-exports for convenience
#[cfg(feature = "stats")]
pub use aggregator::{AggregatedStats, Aggregator, HistoricalMetricsSummary};
pub use collector::GlobalStats;
#[cfg(feature = "stats")]
pub use collector::HistogramStats;
#[cfg(feature = "stats")]
pub use collector::StatsCollector;
pub use config::{
    AlertConfig, HistogramConfig, MonitoringConfig, PerformanceImpact, StatsConfig, TrackedMetric,
    TrackingConfig, TrackingLevel,
};
pub use counter::{Counter, CounterType};
#[cfg(feature = "stats")]
pub use export::{ExportFormat, StatsExporter};
#[cfg(feature = "stats")]
pub use histogram::{HistogramData, MemoryHistogram, Percentile};
pub use memory_stats::{MemoryMetrics, MemoryStats};
#[cfg(feature = "stats")]
pub use predictive::{MemoryTrend, Prediction, PredictionModel, PredictiveAnalytics, TrendType};
#[cfg(feature = "profiling")]
pub use profiler::{AllocationSite, HotSpot, MemoryProfiler, ProfileReport};
#[cfg(feature = "stats")]
pub use real_time::{MemoryAlert, RealTimeData, RealTimeMonitor};
#[cfg(feature = "stats")]
pub use snapshot::{MemorySnapshot, SnapshotDiff, SnapshotFormat};
#[cfg(feature = "stats")]
pub use tracker::{DataPoint, MemoryTracker, WindowStats};

/// Initialize global statistics system
pub fn initialize(config: StatsConfig) -> crate::error::MemoryResult<()> {
    config
        .validate()
        .map_err(|_e| crate::error::MemoryError::invalid_config("invalid config"))?;
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
