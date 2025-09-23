//! Historical memory tracking and trend analysis
//!
//! This module provides `MemoryTracker` which maintains historical snapshots
//! of memory metrics for trend analysis and debugging.

#[cfg(not(feature = "std"))]
use alloc::collections::VecDeque;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use core::fmt;
#[cfg(feature = "std")]
use std::collections::VecDeque;
#[cfg(feature = "std")]
use std::time::{Duration, Instant};

#[cfg(feature = "std")]
use parking_lot::RwLock;

use super::config::{TrackedMetric, TrackingConfig, TrackingLevel};
use super::memory_stats::MemoryMetrics;

/// Time series data point
#[derive(Debug, Clone)]
pub struct DataPoint {
    #[cfg(feature = "std")]
    pub timestamp: Instant,
    pub value: f64,
    pub metadata: Option<&'static str>,
}

/// Window statistics
#[derive(Debug, Clone)]
pub struct WindowStats {
    #[cfg(feature = "std")]
    pub window: Duration,
    pub average_usage: f64,
    pub max_usage: usize,
    pub min_usage: usize,
    pub total_allocations: u64,
    pub samples: usize,
}

/// Memory tracker for historical analysis
#[cfg(feature = "std")]
pub struct MemoryTracker {
    config: TrackingConfig,
    history: RwLock<VecDeque<MemoryMetrics>>,
    metric_history: RwLock<hashbrown::HashMap<TrackedMetric, VecDeque<DataPoint>>>,
    last_sample: RwLock<Instant>,
}

#[cfg(feature = "std")]
impl MemoryTracker {
    /// Create new memory tracker
    pub fn new(config: TrackingConfig) -> Self {
        let mut metric_history = hashbrown::HashMap::new();
        let max_history = config.max_history;

        // Initialize history for each tracked metric
        for &metric in &config.tracked_metrics {
            metric_history.insert(metric, VecDeque::with_capacity(max_history));
        }

        Self {
            history: RwLock::new(VecDeque::with_capacity(max_history)),
            metric_history: RwLock::new(metric_history),
            last_sample: RwLock::new(Instant::now()),
            config,
        }
    }

    /// Add a metrics snapshot to history
    pub fn add_snapshot(&self, metrics: MemoryMetrics) {
        if self.config.level == TrackingLevel::Disabled {
            return;
        }

        let now = Instant::now();

        // Add to main history
        {
            let mut history = self.history.write();
            if history.len() >= self.config.max_history && self.config.max_history > 0 {
                history.pop_front();
            }
            if self.config.max_history > 0 {
                history.push_back(metrics.clone());
            }
        }

        // Add to per-metric history
        {
            let mut metric_history = self.metric_history.write();

            for &metric_type in &self.config.tracked_metrics {
                let value = match metric_type {
                    TrackedMetric::CurrentUsage => metrics.current_allocated as f64,
                    TrackedMetric::PeakUsage => metrics.peak_allocated as f64,
                    TrackedMetric::AllocationRate => metrics.allocation_rate(),
                    TrackedMetric::FragmentationPercentage => metrics.fragmentation_ratio() * 100.0,
                    TrackedMetric::AllocationLatency => metrics.avg_allocation_latency_nanos(),
                    TrackedMetric::Custom(_) => 0.0, // Placeholder for custom metrics
                };

                if let Some(history) = metric_history.get_mut(&metric_type) {
                    if history.len() >= self.config.max_history && self.config.max_history > 0 {
                        history.pop_front();
                    }
                    if self.config.max_history > 0 {
                        history.push_back(DataPoint { timestamp: now, value, metadata: None });
                    }
                }
            }
        }

        *self.last_sample.write() = now;
    }

    /// Check if we should sample based on interval
    pub fn should_sample(&self) -> bool {
        if self.config.level == TrackingLevel::Disabled {
            return false;
        }

        let now = Instant::now();
        let last = *self.last_sample.read();
        now.duration_since(last) >= self.config.sampling_interval
    }

    /// Get historical snapshots
    pub fn get_history(&self) -> Vec<MemoryMetrics> {
        self.history.read().iter().cloned().collect()
    }

    /// Get history for a specific metric
    pub fn get_metric_history(&self, metric: TrackedMetric) -> Vec<DataPoint> {
        self.metric_history
            .read()
            .get(&metric)
            .map(|deque| deque.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Calculate statistics over a time window
    pub fn calculate_window_stats(&self, window: Duration) -> Option<WindowStats> {
        let history = self.history.read();
        if history.is_empty() {
            return None;
        }

        let cutoff_time = Instant::now() - window;
        let window_data: Vec<&MemoryMetrics> =
            history.iter().filter(|metrics| metrics.timestamp >= cutoff_time).collect();

        if window_data.is_empty() {
            return None;
        }

        let avg_usage = window_data.iter().map(|m| m.current_allocated as f64).sum::<f64>()
            / window_data.len() as f64;

        let max_usage = window_data.iter().map(|m| m.current_allocated).max().unwrap_or(0);

        let min_usage = window_data.iter().map(|m| m.current_allocated).min().unwrap_or(0);

        let total_allocations = window_data
            .last()
            .map(|m| m.allocations)
            .unwrap_or(0)
            .saturating_sub(window_data.first().map(|m| m.allocations).unwrap_or(0));

        Some(WindowStats {
            window,
            average_usage: avg_usage,
            max_usage,
            min_usage,
            total_allocations,
            samples: window_data.len(),
        })
    }

    /// Get trend analysis for a metric
    pub fn analyze_trend(&self, metric: TrackedMetric) -> Option<TrendAnalysis> {
        let data = self.get_metric_history(metric);
        if data.len() < 3 {
            return None;
        }

        // Simple linear regression
        let n = data.len() as f64;
        let mut sum_x = 0.0;
        let mut sum_y = 0.0;
        let mut sum_xy = 0.0;
        let mut sum_xx = 0.0;

        let start_time = data[0].timestamp;
        for point in data.iter() {
            let x = point.timestamp.duration_since(start_time).as_secs_f64();
            let y = point.value;

            sum_x += x;
            sum_y += y;
            sum_xy += x * y;
            sum_xx += x * x;
        }

        let slope = (n * sum_xy - sum_x * sum_y) / (n * sum_xx - sum_x * sum_x);
        let intercept = (sum_y - slope * sum_x) / n;

        // Calculate R-squared
        let mean_y = sum_y / n;
        let ss_tot: f64 = data.iter().map(|p| (p.value - mean_y).powi(2)).sum();
        let ss_res: f64 = data
            .iter()
            .map(|p| {
                let x = p.timestamp.duration_since(start_time).as_secs_f64();
                let predicted = slope * x + intercept;
                (p.value - predicted).powi(2)
            })
            .sum();

        let r_squared = if ss_tot > 0.0 { 1.0 - (ss_res / ss_tot) } else { 0.0 };

        let trend_type = if slope.abs() < 0.01 {
            TrendType::Stable
        } else if slope > 0.0 {
            TrendType::Increasing
        } else {
            TrendType::Decreasing
        };

        Some(TrendAnalysis {
            metric,
            slope,
            intercept,
            r_squared,
            trend_type,
            confidence: r_squared,
        })
    }

    /// Reset tracking history
    pub fn reset(&self) {
        self.history.write().clear();
        self.metric_history.write().iter_mut().for_each(|(_, history)| {
            history.clear();
        });
        *self.last_sample.write() = Instant::now();
    }

    /// Get configuration
    pub fn config(&self) -> &TrackingConfig {
        &self.config
    }

    /// Check if tracking is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.level != TrackingLevel::Disabled
    }
}

/// Trend analysis result
#[derive(Debug, Clone)]
pub struct TrendAnalysis {
    pub metric: TrackedMetric,
    pub slope: f64,
    pub intercept: f64,
    pub r_squared: f64,
    pub trend_type: TrendType,
    pub confidence: f64,
}

/// Type of trend detected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendType {
    Stable,
    Increasing,
    Decreasing,
    Volatile,
}

impl TrendAnalysis {
    /// Predict value at future time
    pub fn predict(&self, future_time: Duration) -> f64 {
        let x = future_time.as_secs_f64();
        self.slope * x + self.intercept
    }

    /// Check if trend is significant
    pub fn is_significant(&self) -> bool {
        self.r_squared > 0.5 && self.confidence > 0.7
    }
}

impl fmt::Display for TrendAnalysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Trend for {:?}: {:?} (slope: {:.2}, R²: {:.2}, confidence: {:.2})",
            self.metric, self.trend_type, self.slope, self.r_squared, self.confidence
        )
    }
}

#[cfg(not(feature = "std"))]
pub struct MemoryTracker {
    // Minimal no_std implementation
    config: TrackingConfig,
}

#[cfg(not(feature = "std"))]
impl MemoryTracker {
    pub fn new(config: TrackingConfig) -> Self {
        Self { config }
    }

    pub fn add_snapshot(&self, _metrics: MemoryMetrics) {
        // No-op in no_std
    }

    pub fn should_sample(&self) -> bool {
        false
    }

    pub fn get_history(&self) -> Vec<MemoryMetrics> {
        Vec::new()
    }

    pub fn reset(&self) {
        // No-op
    }

    pub fn is_enabled(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::config::SamplingConfig;

    fn create_test_config() -> TrackingConfig {
        TrackingConfig {
            level: TrackingLevel::Basic,
            max_history: 100,
            sampling_interval: Duration::from_millis(100),
            tracked_metrics: vec![TrackedMetric::CurrentUsage, TrackedMetric::AllocationRate],
            detailed_tracking: false,
            sampling: SamplingConfig::light(),
            #[cfg(feature = "profiling")]
            collect_stack_traces: false,
            #[cfg(feature = "profiling")]
            profiler_sampling_rate: 0.0,
            #[cfg(feature = "profiling")]
            profiler_size_threshold: 0,
            #[cfg(feature = "profiling")]
            max_stack_depth: 0,
        }
    }

    fn create_test_metrics(current: usize, allocs: u64) -> MemoryMetrics {
        let mut metrics = MemoryMetrics::default();
        metrics.current_allocated = current;
        metrics.allocations = allocs;
        metrics.timestamp = Instant::now();
        metrics
    }

    #[test]
    fn test_tracker_creation() {
        let config = create_test_config();
        let tracker = MemoryTracker::new(config);

        assert!(tracker.is_enabled());
        assert_eq!(tracker.get_history().len(), 0);
    }

    #[test]
    fn test_add_snapshots() {
        let config = create_test_config();
        let tracker = MemoryTracker::new(config);

        // Add some snapshots
        tracker.add_snapshot(create_test_metrics(1000, 10));
        tracker.add_snapshot(create_test_metrics(1200, 15));
        tracker.add_snapshot(create_test_metrics(1100, 20));

        let history = tracker.get_history();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].current_allocated, 1000);
        assert_eq!(history[2].current_allocated, 1100);
    }

    #[test]
    fn test_metric_history() {
        let config = create_test_config();
        let tracker = MemoryTracker::new(config);

        tracker.add_snapshot(create_test_metrics(1000, 10));
        tracker.add_snapshot(create_test_metrics(1200, 15));

        let usage_history = tracker.get_metric_history(TrackedMetric::CurrentUsage);
        assert_eq!(usage_history.len(), 2);
        assert_eq!(usage_history[0].value, 1000.0);
        assert_eq!(usage_history[1].value, 1200.0);
    }

    #[test]
    fn test_trend_analysis() {
        let config = create_test_config();
        let tracker = MemoryTracker::new(config);

        // Add increasing trend
        for i in 0..10 {
            tracker.add_snapshot(create_test_metrics(1000 + i * 100, (10 + i) as u64));
            std::thread::sleep(Duration::from_millis(10));
        }

        let trend = tracker.analyze_trend(TrackedMetric::CurrentUsage);
        assert!(trend.is_some());

        let trend = trend.unwrap();
        assert_eq!(trend.trend_type, TrendType::Increasing);
        assert!(trend.slope > 0.0);
    }

    #[test]
    fn test_window_stats() {
        let config = create_test_config();
        let tracker = MemoryTracker::new(config);

        // Add some historical data
        tracker.add_snapshot(create_test_metrics(1000, 10));
        std::thread::sleep(Duration::from_millis(50));
        tracker.add_snapshot(create_test_metrics(1200, 15));
        std::thread::sleep(Duration::from_millis(50));
        tracker.add_snapshot(create_test_metrics(800, 20));

        let stats = tracker.calculate_window_stats(Duration::from_millis(200));
        assert!(stats.is_some());

        let stats = stats.unwrap();
        assert_eq!(stats.samples, 3);
        assert_eq!(stats.max_usage, 1200);
        assert_eq!(stats.min_usage, 800);
        assert!((stats.average_usage - 1000.0).abs() < 1.0);
    }

    #[test]
    fn test_history_capacity() {
        let mut config = create_test_config();
        config.max_history = 3;
        let tracker = MemoryTracker::new(config);

        // Add more snapshots than capacity
        for i in 0..5 {
            tracker.add_snapshot(create_test_metrics(1000 + i * 100, (10 + i) as u64));
        }

        let history = tracker.get_history();
        assert_eq!(history.len(), 3);
        // Should contain the last 3 snapshots
        assert_eq!(history[0].current_allocated, 1200); // 1000 + 2*100
        assert_eq!(history[2].current_allocated, 1400); // 1000 + 4*100
    }

    #[test]
    fn test_disabled_tracking() {
        let mut config = create_test_config();
        config.level = TrackingLevel::Disabled;
        let tracker = MemoryTracker::new(config);

        assert!(!tracker.is_enabled());

        tracker.add_snapshot(create_test_metrics(1000, 10));
        assert_eq!(tracker.get_history().len(), 0);
    }
}
