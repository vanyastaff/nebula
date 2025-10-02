//! Aggregates memory statistics and reports from various components
//! to provide a comprehensive view of memory usage.

#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use std::sync::Arc;
#[cfg(feature = "std")]
use std::time::Duration;

#[cfg(feature = "std")]
use parking_lot::RwLock;

use super::config::StatsConfig; // Use general StatsConfig for overall enablement
use super::memory_stats::{MemoryMetrics, MemoryStats};
use super::predictive::{MemoryTrend, Prediction, PredictiveAnalytics};
#[cfg(feature = "profiling")]
use super::profiler::{MemoryProfiler, ProfileReport}; // Only if profiling feature is enabled
use super::real_time::{MemoryAlert, RealTimeData, RealTimeMonitor}; /* Real-time data and
 * alerts */
use super::tracker::MemoryTracker; // Tracker provides historical data // For predictions
// and trends

/// A comprehensive snapshot of all aggregated memory statistics.
#[derive(Debug, Clone)]
#[cfg(feature = "std")]
pub struct AggregatedStats {
    pub timestamp: std::time::Instant,
    pub overall_metrics: MemoryMetrics,
    #[cfg(feature = "profiling")]
    pub profile_report: Option<ProfileReport>, // Only if profiling is enabled
    pub latest_live_data: Option<RealTimeData>, // Latest from real-time monitor
    pub historical_metrics_summary: Option<HistoricalMetricsSummary>,
    pub memory_trend: Option<MemoryTrend>,     // Trend analysis
    pub future_prediction: Option<Prediction>, // Future memory usage prediction
    pub active_alerts: Vec<MemoryAlert>,       // All active alerts from the real-time monitor
}

/// Summarized historical data for long-term analysis.
#[derive(Debug, Clone)]
#[cfg(feature = "std")]
pub struct HistoricalMetricsSummary {
    pub total_snapshots: usize,
    pub avg_current_allocated: f64,
    pub max_peak_allocated: usize,
    pub avg_allocation_rate: f64,
    pub total_allocations: u64,
    pub fragmentation_over_time: f64, // Average fragmentation
    pub period_duration: Duration,    /* Duration covered by the historical data
                                       * Add more summaries (e.g., histogram data over time) */
}

/// The main aggregator responsible for collecting data from various memory
/// components.
#[cfg(feature = "std")]
pub struct Aggregator {
    config: StatsConfig,
    monitored_stats: Arc<MemoryStats>,
    tracker: Arc<MemoryTracker>,
    #[cfg(feature = "profiling")]
    profiler: Arc<RwLock<MemoryProfiler>>,
    monitor: Arc<RwLock<RealTimeMonitor>>, // RealTimeMonitor needs to be mutable and shareable
    predictive_analytics: Arc<RwLock<PredictiveAnalytics>>, // For trend and prediction
}

#[cfg(feature = "std")]
impl Aggregator {
    /// Creates a new `Aggregator` instance.
    ///
    /// # Arguments
    /// * `config` - The overall `StatsConfig` that governs all components.
    /// * `monitored_stats` - `Arc` to the core `MemoryStats` instance.
    /// * `tracker` - `Arc` to the `MemoryTracker` instance.
    /// * `profiler` - `Arc` to the `MemoryProfiler` instance (conditionally
    ///   compiled).
    /// * `monitor` - `Arc<RwLock>` to the `RealTimeMonitor` instance.
    /// * `predictive_analytics` - `Arc<RwLock>` to the `PredictiveAnalytics`
    ///   instance.
    pub fn new(
        config: StatsConfig,
        monitored_stats: Arc<MemoryStats>,
        tracker: Arc<MemoryTracker>,
        #[cfg(feature = "profiling")] profiler: Arc<RwLock<MemoryProfiler>>,
        monitor: Arc<RwLock<RealTimeMonitor>>,
        predictive_analytics: Arc<RwLock<PredictiveAnalytics>>,
    ) -> Self {
        Self {
            config,
            monitored_stats,
            tracker,
            #[cfg(feature = "profiling")]
            profiler,
            monitor,
            predictive_analytics,
        }
    }

    /// Collects and aggregates the latest memory statistics from all active
    /// components.
    pub fn collect_and_aggregate(&self) -> AggregatedStats {
        let now = std::time::Instant::now();

        // 1. Get Overall Memory Metrics from MemoryStats
        let overall_metrics = self.monitored_stats.metrics();

        // 2. Get Profile Report from MemoryProfiler (if enabled)
        #[cfg(feature = "profiling")]
        let profile_report = if self.profiler.read().is_profiling_enabled() {
            Some(
                self.profiler
                    .read()
                    .generate_report(overall_metrics.clone()),
            )
        } else {
            None
        };

        // 3. Get Latest Live Data from RealTimeMonitor (if running)
        let latest_live_data = self.monitor.read().get_latest_data();
        let active_alerts = latest_live_data
            .as_ref()
            .map_or(Vec::new(), |data| data.active_alerts.clone());

        // 4. Summarize Historical Metrics from MemoryTracker (if history is enabled)
        let historical_metrics_summary = if self.config.tracking.max_history > 0 {
            let history = self.tracker.get_history();
            if !history.is_empty() {
                let total_snapshots = history.len();
                let avg_current_allocated = history
                    .iter()
                    .map(|m| m.current_allocated as f64)
                    .sum::<f64>()
                    / total_snapshots as f64;
                let max_peak_allocated =
                    history.iter().map(|m| m.peak_allocated).max().unwrap_or(0); // Safe unwrap due to !is_empty()
                let avg_allocation_rate = history
                    .iter()
                    .filter(|m| m.elapsed_secs > 0.0) // Avoid division by zero
                    .map(|m| m.allocation_rate())
                    .sum::<f64>()
                    / history
                        .iter()
                        .filter(|m| m.elapsed_secs > 0.0)
                        .count()
                        .max(1) as f64; // At least 1 for division
                let total_allocations = history.last().map_or(0, |m| m.allocations); // Total since tracker started
                let fragmentation_over_time =
                    history.iter().map(|m| m.fragmentation_ratio()).sum::<f64>()
                        / total_snapshots as f64;
                let period_duration = if history.len() > 1 {
                    // This is an approximation; ideally, calculate from snapshot timestamps
                    history
                        .last()
                        .unwrap()
                        .timestamp
                        .duration_since(history.first().unwrap().timestamp)
                } else {
                    Duration::ZERO
                };

                Some(HistoricalMetricsSummary {
                    total_snapshots,
                    avg_current_allocated,
                    max_peak_allocated,
                    avg_allocation_rate,
                    total_allocations,
                    fragmentation_over_time,
                    period_duration,
                })
            } else {
                None
            }
        } else {
            None
        };

        // 5. Get Prediction and Trend from PredictiveAnalytics (if enabled)
        let memory_trend = if self.config.analytics.enable_trends {
            self.predictive_analytics.read().analyze_trend()
        } else {
            None
        };

        let future_prediction = if self.config.analytics.enable_predictions {
            // Predict for a fixed horizon, e.g., 5 minutes into the future
            self.predictive_analytics
                .read()
                .predict(self.config.analytics.ml_config.prediction_horizon)
        } else {
            None
        };

        AggregatedStats {
            timestamp: now,
            overall_metrics,
            #[cfg(feature = "profiling")]
            profile_report,
            latest_live_data,
            historical_metrics_summary,
            memory_trend,
            future_prediction,
            active_alerts,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::stats::config::{AnalyticsConfig, MLConfig, MLModelType, StatsConfig};
    use crate::stats::memory_stats::MemoryStats;
    #[cfg(feature = "profiling")]
    use crate::stats::profiler::{AllocationSite, MemoryProfiler};
    use crate::stats::real_time::RealTimeMonitor;
    use crate::stats::tracker::{DataPoint, MemoryTracker};

    // Helper to create a dummy MemoryStats instance for tests
    fn create_dummy_memory_stats(
        allocations: u64,
        deallocations: u64,
        current_allocated: usize,
        peak_allocated: usize,
        total_allocated_bytes: usize,
        total_deallocated_bytes: usize,
    ) -> Arc<MemoryStats> {
        let stats = MemoryStats::new();
        stats.allocations.store(allocations, Ordering::Relaxed);
        stats.deallocations.store(deallocations, Ordering::Relaxed);
        stats
            .allocated_bytes
            .store(current_allocated, Ordering::Relaxed);
        stats
            .peak_allocated
            .store(peak_allocated, Ordering::Relaxed);
        stats
            .total_allocated_bytes
            .store(total_allocated_bytes, Ordering::Relaxed);
        stats
            .total_deallocated_bytes
            .store(total_deallocated_bytes, Ordering::Relaxed);
        #[cfg(feature = "std")]
        stats
            .total_allocation_time_nanos
            .store(0, Ordering::Relaxed);
        Arc::new(stats)
    }

    // Helper to create a basic AnalyticsConfig for tests
    fn create_basic_analytics_config(
        enable_predictions: bool,
        enable_trends: bool,
    ) -> AnalyticsConfig {
        AnalyticsConfig {
            enable_predictions,
            enable_trends,
            enable_anomaly_detection: false,
            ml_config: MLConfig {
                model_type: MLModelType::LinearRegression,
                training_window_size: 3, // Small window for tests
                prediction_horizon: Duration::from_secs(10),
                confidence_threshold: 0.8,
            },
        }
    }

    #[test]
    fn test_aggregator_new() {
        let stats_config = StatsConfig::basic();
        let monitored_stats = create_dummy_memory_stats(0, 0, 0, 0, 0, 0);
        let tracker = Arc::new(MemoryTracker::new(stats_config.tracking.clone()));
        #[cfg(feature = "profiling")]
        let profiler = Arc::new(RwLock::new(MemoryProfiler::new(
            stats_config.tracking.clone(),
        )));
        let monitor = Arc::new(RwLock::new(RealTimeMonitor::new(
            stats_config.monitoring.clone(),
            stats_config.alerts.clone(),
            monitored_stats.clone(),
        )));
        let predictive_analytics = Arc::new(RwLock::new(PredictiveAnalytics::new(
            stats_config.analytics.ml_config.model_type.into(),
            stats_config.tracking.max_history,
        )));

        let _aggregator = Aggregator::new(
            stats_config,
            monitored_stats,
            tracker,
            #[cfg(feature = "profiling")]
            profiler,
            monitor,
            predictive_analytics,
        );
        // If it compiles and runs without panic, new is successful
    }

    #[test]
    fn test_aggregator_collect_and_aggregate_basic() {
        let mut stats_config = StatsConfig::basic();
        stats_config.monitoring.enabled = false; // Disable monitor for simpler test
        stats_config.alerts.enabled = false; // Disable alerts
        stats_config.tracking.max_history = 10; // Enable history
        stats_config.analytics = create_basic_analytics_config(false, false); // Disable analytics for now

        let monitored_stats = create_dummy_memory_stats(100, 50, 500, 1000, 5000, 4500);
        let tracker = Arc::new(MemoryTracker::new(stats_config.tracking.clone()));
        #[cfg(feature = "profiling")]
        let profiler = Arc::new(RwLock::new(MemoryProfiler::new(
            stats_config.tracking.clone(),
        )));
        let monitor = Arc::new(RwLock::new(RealTimeMonitor::new(
            stats_config.monitoring.clone(),
            stats_config.alerts.clone(),
            monitored_stats.clone(),
        )));
        let predictive_analytics = Arc::new(RwLock::new(PredictiveAnalytics::new(
            stats_config.analytics.ml_config.model_type.into(),
            stats_config.tracking.max_history,
        )));

        // Simulate some data in tracker for historical summary
        for i in 0..5 {
            // Добавляем метрики в трекер используя правильный метод add_snapshot
            let metrics = monitored_stats.as_ref().metrics();
            tracker.as_ref().add_snapshot(metrics);

            // For testing, we need to ensure stats change or mock data points for tracker
            monitored_stats
                .allocated_bytes
                .store(500 + i * 100, Ordering::Relaxed);
            monitored_stats
                .peak_allocated
                .store(1000 + i * 100, Ordering::Relaxed);
            monitored_stats
                .allocations
                .store(100 + i as u64, Ordering::Relaxed);
        }

        let aggregator = Aggregator::new(
            stats_config,
            monitored_stats,
            tracker,
            #[cfg(feature = "profiling")]
            profiler,
            monitor,
            predictive_analytics,
        );

        let aggregated_stats = aggregator.collect_and_aggregate();

        // Verify overall metrics
        assert_eq!(aggregated_stats.overall_metrics.current_allocated, 900); // Last updated value (500 + 4*100)
        assert_eq!(aggregated_stats.overall_metrics.allocations, 104); // Last updated value (100 + 4)

        // Verify profile report (should be None if profiling not enabled or no
        // activity)
        #[cfg(feature = "profiling")]
        assert!(aggregated_stats.profile_report.is_none());

        // Verify latest live data (should be None as monitor is disabled for this test)
        assert!(aggregated_stats.latest_live_data.is_none());
        assert!(aggregated_stats.active_alerts.is_empty());

        // Verify historical summary (should be Some as max_history > 0)
        assert!(aggregated_stats.historical_metrics_summary.is_some());
        let hist_summary = aggregated_stats.historical_metrics_summary.unwrap();
        assert_eq!(hist_summary.total_snapshots, 5);
        assert!(hist_summary.avg_current_allocated > 0.0);
        assert!(hist_summary.max_peak_allocated > 0);
        assert!(hist_summary.total_allocations > 0);

        // Verify analytics (should be None as analytics is disabled)
        assert!(aggregated_stats.memory_trend.is_none());
        assert!(aggregated_stats.future_prediction.is_none());
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn test_aggregator_collect_and_aggregate_with_profiling() {
        let mut stats_config = StatsConfig::debug(); // Enable detailed tracking which includes profiling
        stats_config.monitoring.enabled = false;
        stats_config.alerts.enabled = false;
        stats_config.analytics = create_basic_analytics_config(false, false);

        let monitored_stats = create_dummy_memory_stats(0, 0, 0, 0, 0, 0);
        let tracker = Arc::new(MemoryTracker::new(stats_config.tracking.clone()));
        let profiler = Arc::new(RwLock::new(MemoryProfiler::new(
            stats_config.tracking.clone(),
        )));
        let monitor = Arc::new(RwLock::new(RealTimeMonitor::new(
            stats_config.monitoring.clone(),
            stats_config.alerts.clone(),
            monitored_stats.clone(),
        )));
        let predictive_analytics = Arc::new(RwLock::new(PredictiveAnalytics::new(
            stats_config.analytics.ml_config.model_type.into(),
            stats_config.tracking.max_history,
        )));

        // Simulate some allocations that the profiler would capture
        // We need to pass a dummy AllocationSite directly for the profiler's record
        // methods in tests
        let site1 = AllocationSite::new_manual("test_file.rs", 10, "test_func_a");
        let site2 = AllocationSite::new_manual("another_file.rs", 20, "test_func_b");

        // У Arc<RwLock<MemoryProfiler>> нужно использовать write() для получения
        // изменяемого доступа к методам, требующим &mut self
        let mut profiler_guard = profiler.write();
        profiler_guard.record_allocation_event(100, Some(Duration::from_nanos(10)));
        profiler_guard.record_allocation_event(200, Some(Duration::from_nanos(20)));
        profiler_guard.record_allocation_event(50, Some(Duration::from_nanos(5)));
        drop(profiler_guard); // Освобождаем блокировку

        let aggregator = Aggregator::new(
            stats_config,
            monitored_stats,
            tracker,
            profiler,
            monitor,
            predictive_analytics,
        );

        let aggregated_stats = aggregator.collect_and_aggregate();

        // Verify profile report is present and has data
        assert!(aggregated_stats.profile_report.is_some());
        let report = aggregated_stats.profile_report.unwrap();
        assert!(!report.hot_spots.is_empty());
        assert!(report.total_sampled > 0);
    }

    #[test]
    fn test_aggregator_collect_and_aggregate_with_monitor_and_alerts() {
        let mut stats_config = StatsConfig::basic();
        stats_config.monitoring.enabled = true;
        stats_config.monitoring.interval = Duration::from_millis(50); // Make monitor fast for test
        stats_config.monitoring.collect_histograms = true;
        stats_config.monitoring.histogram_buckets = 10;
        stats_config.alerts.enabled = true;
        stats_config.alerts.memory_threshold = Some(500); // Set a low memory alert threshold
        stats_config.alerts.cooldown = Duration::from_secs(0); // No cooldown for easier testing
        stats_config.analytics = create_basic_analytics_config(false, false);

        let monitored_stats = create_dummy_memory_stats(0, 0, 0, 0, 0, 0);
        let tracker = Arc::new(MemoryTracker::new(stats_config.tracking.clone()));
        #[cfg(feature = "profiling")]
        let profiler = Arc::new(RwLock::new(MemoryProfiler::new(
            stats_config.tracking.clone(),
        )));
        let monitor = Arc::new(RwLock::new(RealTimeMonitor::new(
            stats_config.monitoring.clone(),
            stats_config.alerts.clone(),
            monitored_stats.clone(),
        )));
        let predictive_analytics = Arc::new(RwLock::new(PredictiveAnalytics::new(
            stats_config.analytics.ml_config.model_type.into(),
            stats_config.tracking.max_history,
        )));

        // Start the real-time monitor thread
        monitor.write().start().unwrap();

        // Simulate activity that triggers an alert and histogram samples
        monitored_stats
            .allocated_bytes
            .store(600, Ordering::Relaxed); // Trigger memory alert
        monitored_stats.allocations.store(100, Ordering::Relaxed);
        monitor.write().add_histogram_sample(10);
        monitor.write().add_histogram_sample(200);

        std::thread::sleep(Duration::from_millis(100)); // Allow monitor thread to run and collect

        let aggregator = Aggregator::new(
            stats_config,
            monitored_stats,
            tracker,
            #[cfg(feature = "profiling")]
            profiler,
            monitor.clone(), // Use clone to pass to aggregator and still retain for stop()
            predictive_analytics,
        );

        let aggregated_stats = aggregator.collect_and_aggregate();

        // Verify latest live data and alerts
        assert!(aggregated_stats.latest_live_data.is_some());
        let live_data = aggregated_stats.latest_live_data.unwrap();
        assert_eq!(live_data.metrics.current_allocated, 600);
        assert!(!live_data.active_alerts.is_empty());
        assert_eq!(live_data.active_alerts[0].name, "High Memory Usage");
        assert!(live_data.histogram.is_some());
        assert_eq!(live_data.histogram.unwrap().total_samples, 2);

        // Stop the monitor thread
        monitor.write().stop();
    }

    #[test]
    fn test_aggregator_collect_and_aggregate_with_analytics() {
        let mut stats_config = StatsConfig::basic();
        stats_config.monitoring.enabled = false;
        stats_config.alerts.enabled = false;
        stats_config.tracking.max_history = 10;
        stats_config.analytics = create_basic_analytics_config(true, true); // Enable predictions and trends

        let monitored_stats = create_dummy_memory_stats(0, 0, 0, 0, 0, 0);
        let tracker = Arc::new(MemoryTracker::new(stats_config.tracking.clone()));
        #[cfg(feature = "profiling")]
        let profiler = Arc::new(RwLock::new(MemoryProfiler::new(
            stats_config.tracking.clone(),
        )));
        let monitor = Arc::new(RwLock::new(RealTimeMonitor::new(
            stats_config.monitoring.clone(),
            stats_config.alerts.clone(),
            monitored_stats.clone(),
        )));
        let predictive_analytics = Arc::new(RwLock::new(PredictiveAnalytics::new(
            stats_config.analytics.ml_config.model_type.into(),
            stats_config.tracking.max_history,
        )));

        // Add some data points to predictive analytics for trend/prediction
        let base_time = std::time::Instant::now();
        for i in 0..5 {
            // Используем новый Instant для каждой точки, добавляя длительность к базовому
            // времени
            let timestamp = base_time + Duration::from_secs(i as u64 * 10);
            predictive_analytics.write().add_data_point(DataPoint {
                timestamp,
                value: (100.0 + i as f64 * 10.0), // Linear growth
                metadata: None,
            });
        }

        let aggregator = Aggregator::new(
            stats_config,
            monitored_stats,
            tracker,
            #[cfg(feature = "profiling")]
            profiler,
            monitor,
            predictive_analytics,
        );

        let aggregated_stats = aggregator.collect_and_aggregate();

        // Verify analytics results
        assert!(aggregated_stats.memory_trend.is_some());
        assert_eq!(
            aggregated_stats.memory_trend.unwrap().trend_type,
            crate::stats::predictive::TrendType::Growing
        );

        assert!(aggregated_stats.future_prediction.is_some());
        let prediction = aggregated_stats.future_prediction.unwrap();
        // The last data point was 100 + 4*10 = 140. Prediction horizon is 10s.
        // Slope is 1.0 (value per second, assuming DataPoint x is seconds).
        // For linear model, (140 + 10) = 150.0 (if time values are in seconds)
        // With simplified time (0,1,2,3,4) for (100,110,120,130,140) and horizon of 10
        // then slope is 10, intercept is 100. Next x will be 4 + (10/10) = 5
        // Predicted = 10 * 5 + 100 = 150.
        // It's `(base_time + Duration::from_secs(i*10))` vs `future_time = 10s`
        // So x is seconds based on `base_time`.
        // Last data point time is 40s. Predict for 40s + 10s = 50s.
        // Slope is 1.0 (value increase per unit of x, where x is in seconds).
        // Predicted for x=50s: 1.0 * 50.0 + 100.0 (intercept if x_values started from
        // 0) Check this against the `predict_linear` test in `predictive.rs`
        assert!((prediction.value - 150.0).abs() < 5.0); // Allow some floating
        // point variance
    }
}
