//! Configuration for memory statistics collection
//!
//! This module provides hierarchical configuration for all memory statistics,
//! monitoring, and analytics components.

use std::time::Duration;

/// Unified configuration for all memory statistics components
#[derive(Debug, Clone, PartialEq)]
pub struct StatsConfig {
    /// Base tracking configuration
    pub tracking: TrackingConfig,
    /// Real-time monitoring configuration
    pub monitoring: MonitoringConfig,
    /// Alert configuration
    pub alerts: AlertConfig,
    /// Advanced analytics configuration
    pub analytics: AnalyticsConfig,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self::minimal()
    }
}

/// Level of detail for statistics tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackingLevel {
    Disabled,
    Minimal,
    Basic,
    Detailed,
    Debug,
}

/// Metric types that can be tracked
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackedMetric {
    CurrentUsage,
    PeakUsage,
    AllocationRate,
    FragmentationPercentage,
    AllocationLatency,
    Custom(&'static str),
}

/// Configuration for basic memory tracking
#[derive(Debug, Clone, PartialEq)]
pub struct TrackingConfig {
    /// Tracking level
    pub level: TrackingLevel,
    /// Maximum history size
    pub max_history: usize,
    /// Sampling interval
    pub sampling_interval: Duration,
    /// Metrics to track
    pub tracked_metrics: Vec<TrackedMetric>,
    /// Enable detailed tracking
    pub detailed_tracking: bool,
    /// Sampling configuration
    pub sampling: SamplingConfig,
    /// Enable collecting stack traces
    #[cfg(feature = "profiling")]
    pub collect_stack_traces: bool,
    /// Sampling rate for profiling (0.0 to 1.0)
    #[cfg(feature = "profiling")]
    pub profiler_sampling_rate: f64,
    /// Minimum allocation size to profile (in bytes)
    #[cfg(feature = "profiling")]
    pub profiler_size_threshold: usize,
    /// Maximum depth of stack traces to capture
    #[cfg(feature = "profiling")]
    pub max_stack_depth: usize,
}

impl Default for TrackingConfig {
    fn default() -> Self {
        Self::minimal()
    }
}

/// Sampling configuration
#[derive(Debug, Clone, PartialEq)]
pub struct SamplingConfig {
    /// Sampling rate (0.0 to 1.0)
    pub rate: f64,
    /// Enable adaptive sampling
    pub adaptive: bool,
    /// Strategy for sampling
    pub strategy: SamplingStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplingStrategy {
    Random,
    Systematic,
    Adaptive,
    TimeBased,
}

/// Histogram configuration
#[derive(Debug, Clone)]
pub struct HistogramConfig {
    /// Number of buckets
    pub bucket_count: usize,
    /// Minimum value (automatic if None)
    pub min_value: Option<u64>,
    /// Maximum value (automatic if None)
    pub max_value: Option<u64>,
    /// Use logarithmic buckets
    pub logarithmic: bool,
}

impl Default for HistogramConfig {
    fn default() -> Self {
        Self {
            bucket_count: 50,
            min_value: None,
            max_value: None,
            logarithmic: true,
        }
    }
}

/// Real-time monitoring configuration
#[derive(Debug, Clone, PartialEq)]
pub struct MonitoringConfig {
    /// Enable monitoring
    pub enabled: bool,
    /// Monitoring interval
    pub interval: Duration,
    /// Collect histograms
    pub collect_histograms: bool,
    /// Number of histogram buckets
    pub histogram_buckets: usize,
    /// Track component-level stats
    pub component_tracking: bool,
}

/// Alert configuration
#[derive(Debug, Clone, PartialEq)]
pub struct AlertConfig {
    /// Enable alerts
    pub enabled: bool,
    /// Memory threshold in bytes
    pub memory_threshold: Option<u64>,
    /// Allocation rate threshold
    pub allocation_rate_threshold: Option<f64>,
    /// Cooldown period between alerts
    pub cooldown: Duration,
    /// Severity levels
    pub severity_levels: Vec<AlertSeverity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertSeverity {
    pub name: String,
    pub memory_threshold: u64,
    pub allocation_rate_threshold: f64,
}

/// Analytics configuration
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyticsConfig {
    /// Enable predictions
    pub enable_predictions: bool,
    /// Enable trend analysis
    pub enable_trends: bool,
    /// Enable anomaly detection
    pub enable_anomaly_detection: bool,
    /// ML model configuration
    pub ml_config: MLConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MLConfig {
    pub model_type: MLModelType,
    pub training_window_size: usize,
    pub prediction_horizon: Duration,
    pub confidence_threshold: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MLModelType {
    LinearRegression,
    ExponentialSmoothing,
    ARIMA,
    NeuralNetwork,
}

/// Performance impact levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceImpact {
    Minimal, // < 1% overhead
    Low,     // 1-3% overhead
    Medium,  // 3-7% overhead
    High,    // > 7% overhead
}

/// Configuration errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    InvalidSamplingRate,
    InvalidAdaptiveRange,
    ZeroMonitoringInterval,
    InvalidAllocationRate,
    HistoryTooLarge,
    ZeroHistoryWithTrackedMetrics,
    TooManyHistogramBuckets,
    InsufficientHistoryForPredictions,
    InvalidConfidenceThreshold,
    ZeroTrainingWindowSize,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSamplingRate => write!(f, "Sampling rate must be between 0.0 and 1.0"),
            Self::InvalidAdaptiveRange => write!(f, "Invalid adaptive sampling rate range"),
            Self::ZeroMonitoringInterval => {
                write!(f, "Monitoring interval cannot be zero when enabled")
            }
            Self::InvalidAllocationRate => write!(f, "Allocation rate threshold must be positive"),
            Self::HistoryTooLarge => write!(f, "History size too large (max 1,000,000)"),
            Self::ZeroHistoryWithTrackedMetrics => {
                write!(
                    f,
                    "History size cannot be zero if tracked metrics are enabled"
                )
            }
            Self::TooManyHistogramBuckets => write!(f, "Too many histogram buckets (max 1,000)"),
            Self::InsufficientHistoryForPredictions => {
                write!(f, "Insufficient history for predictions (min 100 entries)")
            }
            Self::InvalidConfidenceThreshold => {
                write!(f, "Confidence threshold must be between 0.0 and 1.0")
            }
            Self::ZeroTrainingWindowSize => write!(f, "Training window size cannot be zero"),
        }
    }
}

impl std::error::Error for ConfigError {}

// Implementation blocks with preset configurations
impl StatsConfig {
    /// Minimal configuration with lowest overhead
    pub const fn minimal() -> Self {
        Self {
            tracking: TrackingConfig::minimal(),
            monitoring: MonitoringConfig::disabled(),
            alerts: AlertConfig::disabled(),
            analytics: AnalyticsConfig::disabled(),
        }
    }

    /// Basic configuration for essential stats
    pub fn basic() -> Self {
        Self {
            tracking: TrackingConfig::basic(),
            monitoring: MonitoringConfig::basic(),
            alerts: AlertConfig::disabled(),
            analytics: AnalyticsConfig::disabled(),
        }
    }

    /// Development configuration
    pub fn development() -> Self {
        Self {
            tracking: TrackingConfig::development(),
            monitoring: MonitoringConfig::frequent(),
            alerts: AlertConfig::development(),
            analytics: AnalyticsConfig::basic(),
        }
    }

    /// Production configuration
    pub fn production() -> Self {
        Self {
            tracking: TrackingConfig::production(),
            monitoring: MonitoringConfig::standard(),
            alerts: AlertConfig::production(),
            analytics: AnalyticsConfig::production(),
        }
    }

    /// Debug configuration with maximum detail
    pub fn debug() -> Self {
        Self {
            tracking: TrackingConfig::debug(),
            monitoring: MonitoringConfig::debug(),
            alerts: AlertConfig::development(),
            analytics: AnalyticsConfig::full(),
        }
    }

    /// Validate configuration
    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.tracking.validate()?;
        self.monitoring.validate()?;
        self.alerts.validate()?;
        self.analytics.validate()?;

        // Cross-validation
        if self.analytics.enable_predictions && self.tracking.max_history < 100 {
            return Err(ConfigError::InsufficientHistoryForPredictions);
        }

        Ok(())
    }

    /// Estimate memory overhead
    pub fn estimated_overhead(&self) -> usize {
        self.tracking.memory_usage()
            + self.monitoring.memory_usage()
            + self.alerts.memory_usage()
            + self.analytics.memory_usage()
    }

    /// Calculate performance impact
    pub fn performance_impact(&self) -> PerformanceImpact {
        let score = self.tracking.impact_score()
            + self.monitoring.impact_score()
            + self.analytics.impact_score();

        match score {
            0..=3 => PerformanceImpact::Minimal,
            4..=7 => PerformanceImpact::Low,
            8..=12 => PerformanceImpact::Medium,
            _ => PerformanceImpact::High,
        }
    }
}

impl TrackingConfig {
    pub const fn minimal() -> Self {
        Self {
            level: TrackingLevel::Minimal,
            max_history: 0,
            sampling_interval: Duration::from_secs(0),
            tracked_metrics: Vec::new(),
            detailed_tracking: false,
            sampling: SamplingConfig::disabled(),
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

    pub fn basic() -> Self {
        Self {
            level: TrackingLevel::Basic,
            max_history: 1000,
            sampling_interval: Duration::from_millis(100),
            tracked_metrics: vec![
                TrackedMetric::CurrentUsage,
                TrackedMetric::PeakUsage,
                TrackedMetric::AllocationRate,
            ],
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

    pub fn development() -> Self {
        Self {
            level: TrackingLevel::Detailed,
            max_history: 5000,
            sampling_interval: Duration::from_millis(50),
            tracked_metrics: vec![
                TrackedMetric::CurrentUsage,
                TrackedMetric::PeakUsage,
                TrackedMetric::AllocationRate,
                TrackedMetric::FragmentationPercentage,
                TrackedMetric::AllocationLatency,
            ],
            detailed_tracking: true,
            sampling: SamplingConfig::detailed(),
            #[cfg(feature = "profiling")]
            collect_stack_traces: true,
            #[cfg(feature = "profiling")]
            profiler_sampling_rate: 0.1,
            #[cfg(feature = "profiling")]
            profiler_size_threshold: 1024,
            #[cfg(feature = "profiling")]
            max_stack_depth: 50,
        }
    }

    pub fn production() -> Self {
        Self {
            level: TrackingLevel::Detailed,
            max_history: 2000,
            sampling_interval: Duration::from_millis(200),
            tracked_metrics: vec![
                TrackedMetric::CurrentUsage,
                TrackedMetric::PeakUsage,
                TrackedMetric::AllocationRate,
            ],
            detailed_tracking: true,
            sampling: SamplingConfig::balanced(),
            #[cfg(feature = "profiling")]
            collect_stack_traces: true,
            #[cfg(feature = "profiling")]
            profiler_sampling_rate: 0.01,
            #[cfg(feature = "profiling")]
            profiler_size_threshold: 4096,
            #[cfg(feature = "profiling")]
            max_stack_depth: 20,
        }
    }

    pub fn debug() -> Self {
        Self {
            level: TrackingLevel::Debug,
            max_history: 10000,
            sampling_interval: Duration::from_millis(10),
            tracked_metrics: vec![
                TrackedMetric::CurrentUsage,
                TrackedMetric::PeakUsage,
                TrackedMetric::AllocationRate,
                TrackedMetric::FragmentationPercentage,
                TrackedMetric::AllocationLatency,
            ],
            detailed_tracking: true,
            sampling: SamplingConfig::full(),
            #[cfg(feature = "profiling")]
            collect_stack_traces: true,
            #[cfg(feature = "profiling")]
            profiler_sampling_rate: 1.0,
            #[cfg(feature = "profiling")]
            profiler_size_threshold: 0,
            #[cfg(feature = "profiling")]
            max_stack_depth: 100,
        }
    }

    pub fn memory_usage(&self) -> usize {
        let base = match self.level {
            TrackingLevel::Disabled => 0,
            TrackingLevel::Minimal => self.max_history * 16,
            TrackingLevel::Basic => self.max_history * 32,
            TrackingLevel::Detailed => self.max_history * 64,
            TrackingLevel::Debug => self.max_history * 128,
        };

        let sampling_overhead = self.sampling.memory_usage();

        #[cfg(feature = "profiling")]
        let profiling_overhead = if self.collect_stack_traces {
            // Базовые затраты на сбор стека вызовов
            let base_profiling = 512;
            // Дополнительные затраты на основе глубины стека
            let stack_depth_cost = self.max_stack_depth * 32;
            // Чем ниже порог размера, тем больше затраты (больше аллокаций отслеживается)
            let size_threshold_factor = if self.profiler_size_threshold == 0 {
                4
            } else if self.profiler_size_threshold < 1024 {
                2
            } else {
                1
            };
            // Чем выше частота семплирования, тем больше затраты
            let sampling_factor = (self.profiler_sampling_rate * 10.0) as usize + 1;

            base_profiling + stack_depth_cost * sampling_factor * size_threshold_factor
        } else {
            0
        };

        #[cfg(not(feature = "profiling"))]
        let profiling_overhead = 0;

        base + sampling_overhead + profiling_overhead
    }

    pub fn impact_score(&self) -> u8 {
        let level_score = match self.level {
            TrackingLevel::Disabled => 0,
            TrackingLevel::Minimal => 1,
            TrackingLevel::Basic => 2,
            TrackingLevel::Detailed => 4,
            TrackingLevel::Debug => 6,
        };

        let sampling_score = self.sampling.impact_score();

        #[cfg(feature = "profiling")]
        let profiling_score = if self.collect_stack_traces {
            // Базовый скор для профилирования
            let base_score = 2;
            // Дополнительный скор на основе глубины стека
            let depth_score = if self.max_stack_depth > 50 {
                2
            } else if self.max_stack_depth > 20 {
                1
            } else {
                0
            };
            // Скор на основе порога размера
            let threshold_score = if self.profiler_size_threshold == 0 {
                2
            } else if self.profiler_size_threshold < 1024 {
                1
            } else {
                0
            };
            // Скор на основе частоты семплирования
            let sampling_score = if self.profiler_sampling_rate > 0.5 {
                3
            } else if self.profiler_sampling_rate > 0.1 {
                2
            } else if self.profiler_sampling_rate > 0.01 {
                1
            } else {
                0
            };

            base_score + depth_score + threshold_score + sampling_score
        } else {
            0
        };

        #[cfg(not(feature = "profiling"))]
        let profiling_score = 0;

        level_score + sampling_score + profiling_score
    }

    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_history > 1_000_000 {
            return Err(ConfigError::HistoryTooLarge);
        }
        if self.max_history == 0 && !self.tracked_metrics.is_empty() {
            return Err(ConfigError::ZeroHistoryWithTrackedMetrics);
        }

        #[cfg(feature = "profiling")]
        if self.collect_stack_traces
            && !(0.0..=1.0).contains(&self.profiler_sampling_rate) {
                return Err(ConfigError::InvalidSamplingRate);
            }
            // Другие проверки для профилирования можно добавить здесь

        self.sampling.validate()
    }
}

impl SamplingConfig {
    pub const fn disabled() -> Self {
        Self {
            rate: 0.0,
            adaptive: false,
            strategy: SamplingStrategy::Random,
        }
    }

    pub const fn light() -> Self {
        Self {
            rate: 0.001,
            adaptive: true,
            strategy: SamplingStrategy::Adaptive,
        }
    }

    pub const fn balanced() -> Self {
        Self {
            rate: 0.01,
            adaptive: true,
            strategy: SamplingStrategy::Adaptive,
        }
    }

    pub const fn detailed() -> Self {
        Self {
            rate: 0.1,
            adaptive: false,
            strategy: SamplingStrategy::Random,
        }
    }

    pub const fn full() -> Self {
        Self {
            rate: 1.0,
            adaptive: false,
            strategy: SamplingStrategy::Random,
        }
    }

    pub const fn memory_usage(&self) -> usize {
        if self.adaptive { 512 } else { 64 }
    }

    pub fn impact_score(&self) -> u8 {
        let base = if self.rate > 0.1 {
            4
        } else if self.rate > 0.01 {
            2
        } else if self.rate > 0.0 {
            1
        } else {
            0
        };
        base + if self.adaptive { 1 } else { 0 }
    }

    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(0.0..=1.0).contains(&self.rate) {
            return Err(ConfigError::InvalidSamplingRate);
        }
        Ok(())
    }
}

impl MonitoringConfig {
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(0),
            collect_histograms: false,
            histogram_buckets: 0,
            component_tracking: false,
        }
    }

    pub const fn basic() -> Self {
        Self {
            enabled: true,
            interval: Duration::from_secs(1),
            collect_histograms: false,
            histogram_buckets: 0,
            component_tracking: true,
        }
    }

    pub const fn standard() -> Self {
        Self {
            enabled: true,
            interval: Duration::from_millis(500),
            collect_histograms: true,
            histogram_buckets: 50,
            component_tracking: true,
        }
    }

    pub const fn frequent() -> Self {
        Self {
            enabled: true,
            interval: Duration::from_millis(100),
            collect_histograms: true,
            histogram_buckets: 100,
            component_tracking: true,
        }
    }

    pub const fn debug() -> Self {
        Self {
            enabled: true,
            interval: Duration::from_millis(10),
            collect_histograms: true,
            histogram_buckets: 200,
            component_tracking: true,
        }
    }

    pub const fn memory_usage(&self) -> usize {
        if !self.enabled {
            return 0;
        }

        let histogram_overhead = if self.collect_histograms {
            self.histogram_buckets * 8
        } else {
            0
        };
        let component_overhead = if self.component_tracking { 1024 } else { 0 };
        histogram_overhead + component_overhead
    }

    pub fn impact_score(&self) -> u8 {
        if !self.enabled {
            return 0;
        }

        let interval_score = if self.interval.as_millis() < 100 {
            3
        } else if self.interval.as_millis() < 1000 {
            2
        } else {
            1
        };

        let histogram_score = if self.collect_histograms { 2 } else { 0 };
        let component_score = if self.component_tracking { 1 } else { 0 };

        interval_score + histogram_score + component_score
    }

    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.enabled && self.interval.is_zero() {
            return Err(ConfigError::ZeroMonitoringInterval);
        }
        if self.histogram_buckets > 1000 {
            return Err(ConfigError::TooManyHistogramBuckets);
        }
        Ok(())
    }
}

impl AlertConfig {
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            memory_threshold: None,
            allocation_rate_threshold: None,
            cooldown: Duration::from_secs(0),
            severity_levels: Vec::new(),
        }
    }

    pub const fn development() -> Self {
        Self {
            enabled: true,
            memory_threshold: Some(256 * 1024 * 1024), // 256MB
            allocation_rate_threshold: Some(1000.0),
            cooldown: Duration::from_secs(60),
            severity_levels: Vec::new(),
        }
    }

    pub const fn production() -> Self {
        Self {
            enabled: true,
            memory_threshold: Some(8 * 1024 * 1024 * 1024), // 8GB
            allocation_rate_threshold: Some(50000.0),
            cooldown: Duration::from_secs(300),
            severity_levels: Vec::new(),
        }
    }

    pub const fn memory_usage(&self) -> usize {
        if self.enabled { 256 } else { 0 }
    }

    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(rate) = self.allocation_rate_threshold
            && rate <= 0.0 {
                return Err(ConfigError::InvalidAllocationRate);
            }
        Ok(())
    }
}

impl AnalyticsConfig {
    pub const fn disabled() -> Self {
        Self {
            enable_predictions: false,
            enable_trends: false,
            enable_anomaly_detection: false,
            ml_config: MLConfig::simple(),
        }
    }

    pub const fn basic() -> Self {
        Self {
            enable_predictions: false,
            enable_trends: true,
            enable_anomaly_detection: false,
            ml_config: MLConfig::simple(),
        }
    }

    pub const fn production() -> Self {
        Self {
            enable_predictions: true,
            enable_trends: true,
            enable_anomaly_detection: true,
            ml_config: MLConfig::standard(),
        }
    }

    pub const fn full() -> Self {
        Self {
            enable_predictions: true,
            enable_trends: true,
            enable_anomaly_detection: true,
            ml_config: MLConfig::advanced(),
        }
    }

    pub const fn memory_usage(&self) -> usize {
        let mut overhead = 0;
        if self.enable_predictions {
            overhead += 2048;
        }
        if self.enable_trends {
            overhead += 512;
        }
        if self.enable_anomaly_detection {
            overhead += 1024;
        }
        overhead + self.ml_config.memory_usage()
    }

    pub fn impact_score(&self) -> u8 {
        let mut score = 0;
        if self.enable_predictions {
            score += 3;
        }
        if self.enable_trends {
            score += 1;
        }
        if self.enable_anomaly_detection {
            score += 2;
        }
        score
    }

    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.ml_config.validate()
    }
}

impl MLConfig {
    pub const fn simple() -> Self {
        Self {
            model_type: MLModelType::LinearRegression,
            training_window_size: 100,
            prediction_horizon: Duration::from_secs(300),
            confidence_threshold: 0.8,
        }
    }

    pub const fn standard() -> Self {
        Self {
            model_type: MLModelType::ExponentialSmoothing,
            training_window_size: 500,
            prediction_horizon: Duration::from_secs(900),
            confidence_threshold: 0.85,
        }
    }

    pub const fn advanced() -> Self {
        Self {
            model_type: MLModelType::ARIMA,
            training_window_size: 2000,
            prediction_horizon: Duration::from_secs(1800),
            confidence_threshold: 0.9,
        }
    }

    pub const fn memory_usage(&self) -> usize {
        match self.model_type {
            MLModelType::LinearRegression => 256,
            MLModelType::ExponentialSmoothing => 512,
            MLModelType::ARIMA => 1024,
            MLModelType::NeuralNetwork => 4096,
        }
    }

    #[must_use = "validation result must be checked"]
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(0.0..=1.0).contains(&self.confidence_threshold) {
            return Err(ConfigError::InvalidConfidenceThreshold);
        }
        if self.training_window_size == 0 {
            return Err(ConfigError::ZeroTrainingWindowSize);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_configs() {
        let minimal = StatsConfig::minimal();
        assert_eq!(minimal.performance_impact(), PerformanceImpact::Minimal);

        let debug = StatsConfig::debug();
        assert_eq!(debug.performance_impact(), PerformanceImpact::High);
    }

    #[test]
    fn test_config_validation() {
        let mut config = StatsConfig::basic();
        assert!(config.validate().is_ok());

        config.tracking.max_history = 2_000_000;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_memory_overhead_estimation() {
        let config = StatsConfig::production();
        let overhead = config.estimated_overhead();
        assert!(overhead > 0);
    }
}
