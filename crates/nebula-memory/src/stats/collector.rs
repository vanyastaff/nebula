//! Statistics collector and global statistics types

#[cfg(not(feature = "std"))]
use alloc::{string::String, vec::Vec};

#[cfg(feature = "std")]
use std::sync::Arc;

#[cfg(feature = "std")]
use parking_lot::RwLock;

use super::config::StatsConfig;
use super::memory_stats::{MemoryMetrics, MemoryStats};

#[cfg(all(feature = "stats", feature = "std"))]
use super::aggregator::{AggregatedStats, Aggregator};
#[cfg(feature = "stats")]
use super::histogram::{MemoryHistogram, Percentile};

/// Statistics collector for aggregating memory statistics
///
/// This is the main entry point for collecting and aggregating statistics
/// from various memory subsystems.
#[cfg(all(feature = "stats", feature = "std"))]
pub type StatsCollector = Aggregator;

/// Global system-wide memory statistics
///
/// Provides a comprehensive view of memory usage across all allocators
/// and subsystems in the application.
#[derive(Debug, Clone)]
pub struct GlobalStats {
    /// Overall memory metrics
    pub overall_metrics: MemoryMetrics,

    /// Total allocated memory across all allocators
    pub total_allocated: usize,

    /// Peak allocated memory
    pub peak_allocated: usize,

    /// Total number of allocations
    pub total_allocations: u64,

    /// Total number of deallocations
    pub total_deallocations: u64,

    /// Active allocations (allocations - deallocations)
    pub active_allocations: u64,

    /// Memory fragmentation ratio (0.0 - 1.0)
    pub fragmentation_ratio: f64,

    /// Total allocation failures
    pub allocation_failures: u64,

    /// Average allocation size
    pub avg_allocation_size: f64,

    /// Median allocation size
    pub median_allocation_size: u64,

    /// System-wide allocation rate (allocs/sec)
    pub allocation_rate: f64,

    /// System-wide deallocation rate (deallocs/sec)
    pub deallocation_rate: f64,
}

impl Default for GlobalStats {
    fn default() -> Self {
        Self {
            overall_metrics: MemoryMetrics::default(),
            total_allocated: 0,
            peak_allocated: 0,
            total_allocations: 0,
            total_deallocations: 0,
            active_allocations: 0,
            fragmentation_ratio: 0.0,
            allocation_failures: 0,
            avg_allocation_size: 0.0,
            median_allocation_size: 0,
            allocation_rate: 0.0,
            deallocation_rate: 0.0,
        }
    }
}

impl GlobalStats {
    /// Create new global statistics from memory stats
    pub fn from_memory_stats(stats: &MemoryStats) -> Self {
        let metrics = stats.metrics();

        Self {
            overall_metrics: metrics.clone(),
            total_allocated: metrics.current_allocated,
            peak_allocated: metrics.peak_allocated,
            total_allocations: metrics.allocations,
            total_deallocations: metrics.deallocations,
            active_allocations: metrics.allocations.saturating_sub(metrics.deallocations),
            fragmentation_ratio: 0.0, // MemoryMetrics doesn't have fragmentation field
            allocation_failures: metrics.allocation_failures,
            avg_allocation_size: if metrics.allocations > 0 {
                metrics.current_allocated as f64 / metrics.allocations as f64
            } else {
                0.0
            },
            median_allocation_size: 0, // Would need histogram data
            allocation_rate: 0.0, // Would need time-based tracking
            deallocation_rate: 0.0,
        }
    }

    /// Get memory utilization as percentage (0.0 - 100.0)
    pub fn utilization_percent(&self) -> f64 {
        if self.peak_allocated > 0 {
            (self.total_allocated as f64 / self.peak_allocated as f64) * 100.0
        } else {
            0.0
        }
    }

    /// Check if memory usage is critical (>90% of peak)
    pub fn is_critical(&self) -> bool {
        self.utilization_percent() > 90.0
    }

    /// Check if memory usage is high (>75% of peak)
    pub fn is_high(&self) -> bool {
        self.utilization_percent() > 75.0
    }
}

/// Histogram-based statistics for size distributions
///
/// Provides detailed analysis of allocation size patterns using histograms
/// and percentile calculations.
#[cfg(feature = "stats")]
#[derive(Debug, Clone)]
pub struct HistogramStats {
    /// Allocation size histogram
    pub size_histogram: MemoryHistogram,

    /// Allocation latency histogram (if available)
    #[cfg(feature = "std")]
    pub latency_histogram: Option<MemoryHistogram>,

    /// Cached percentile values
    pub percentiles: Vec<Percentile>,
}

#[cfg(feature = "stats")]
impl HistogramStats {
    /// Create new histogram stats with default configuration
    pub fn new(size_histogram: MemoryHistogram) -> Self {
        let percentiles = size_histogram.percentiles(&[0.5, 0.90, 0.95, 0.99]);

        Self {
            size_histogram,
            #[cfg(feature = "std")]
            latency_histogram: None,
            percentiles,
        }
    }

    /// Update percentiles from current histogram data
    pub fn update_percentiles(&mut self) {
        self.percentiles = self.size_histogram.percentiles(&[0.5, 0.90, 0.95, 0.99]);
    }

    /// Get p50 (median)
    pub fn p50(&self) -> Option<u64> {
        self.percentiles.iter()
            .find(|p| (p.percentile - 0.5).abs() < 0.01)
            .map(|p| p.value)
    }

    /// Get p90 (90th percentile)
    pub fn p90(&self) -> Option<u64> {
        self.percentiles.iter()
            .find(|p| (p.percentile - 0.90).abs() < 0.01)
            .map(|p| p.value)
    }

    /// Get p95 (95th percentile)
    pub fn p95(&self) -> Option<u64> {
        self.percentiles.iter()
            .find(|p| (p.percentile - 0.95).abs() < 0.01)
            .map(|p| p.value)
    }

    /// Get p99 (99th percentile)
    pub fn p99(&self) -> Option<u64> {
        self.percentiles.iter()
            .find(|p| (p.percentile - 0.99).abs() < 0.01)
            .map(|p| p.value)
    }

    /// Get custom percentile
    pub fn percentile(&self, p: f64) -> Option<u64> {
        self.percentiles.iter()
            .find(|percentile| (percentile.percentile - p).abs() < 0.01)
            .map(|percentile| percentile.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_stats_default() {
        let stats = GlobalStats::default();
        assert_eq!(stats.total_allocated, 0);
        assert_eq!(stats.total_allocations, 0);
        assert!(!stats.is_critical());
        assert!(!stats.is_high());
    }

    #[test]
    fn test_global_stats_utilization() {
        let mut stats = GlobalStats::default();
        stats.total_allocated = 800;
        stats.peak_allocated = 1000;

        assert_eq!(stats.utilization_percent(), 80.0);
        assert!(stats.is_high());
        assert!(!stats.is_critical());
    }

    #[test]
    fn test_global_stats_critical() {
        let mut stats = GlobalStats::default();
        stats.total_allocated = 950;
        stats.peak_allocated = 1000;

        assert!(stats.is_critical());
        assert!(stats.is_high());
    }

    #[test]
    #[cfg(feature = "stats")]
    fn test_histogram_stats_percentiles() {
        use super::super::histogram::MemoryHistogram;
        use super::super::config::HistogramConfig;

        let config = HistogramConfig {
            enabled: true,
            bucket_count: 10,
            min_value: Some(0),
            max_value: Some(1000),
            logarithmic: false,
        };

        let histogram = MemoryHistogram::new(config);
        let stats = HistogramStats::new(histogram);

        assert_eq!(stats.percentiles.len(), 4); // p50, p90, p95, p99
    }
}
