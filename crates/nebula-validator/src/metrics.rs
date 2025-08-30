//! Performance metrics and monitoring for the validation framework
//! 
//! This module provides comprehensive metrics collection and monitoring
//! capabilities for tracking validation performance and system health.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn, trace};
use crate::types::{ValidatorId, ValidationComplexity, ErrorCode};

// ==================== Metric Types ====================

/// Metric value types
#[derive(Debug, Clone)]
pub enum MetricValue {
    /// Counter metric (monotonically increasing)
    Counter(u64),
    /// Gauge metric (can go up or down)
    Gauge(f64),
    /// Histogram metric (distribution of values)
    Histogram(HistogramData),
    /// Summary metric (quantiles and counts)
    Summary(SummaryData),
}

/// Histogram data for distribution metrics
#[derive(Debug, Clone)]
pub struct HistogramData {
    /// Bucket boundaries
    pub buckets: Vec<f64>,
    /// Count of values in each bucket
    pub bucket_counts: Vec<u64>,
    /// Sum of all values
    pub sum: f64,
    /// Count of all values
    pub count: u64,
}

/// Summary data for quantile metrics
#[derive(Debug, Clone)]
pub struct SummaryData {
    /// Quantile values (0.0 to 1.0)
    pub quantiles: Vec<f64>,
    /// Values at each quantile
    pub quantile_values: Vec<f64>,
    /// Sum of all values
    pub sum: f64,
    /// Count of all values
    pub count: u64,
}

// ==================== Metric Definition ====================

/// Definition of a metric
#[derive(Debug, Clone)]
pub struct MetricDefinition {
    /// Metric name
    pub name: String,
    /// Metric description
    pub description: String,
    /// Metric type
    pub metric_type: MetricType,
    /// Unit of measurement
    pub unit: Option<String>,
    /// Labels for the metric
    pub labels: Vec<String>,
}

/// Metric types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricType {
    /// Counter metric
    Counter,
    /// Gauge metric
    Gauge,
    /// Histogram metric
    Histogram,
    /// Summary metric
    Summary,
}

// ==================== Metric Labels ====================

/// Labels for metric identification
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetricLabels {
    /// Label key-value pairs
    pub labels: HashMap<String, String>,
}

impl MetricLabels {
    /// Create new metric labels
    pub fn new() -> Self {
        Self {
            labels: HashMap::new(),
        }
    }
    
    /// Add a label
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }
    
    /// Get a label value
    pub fn get(&self, key: &str) -> Option<&String> {
        self.labels.get(key)
    }
    
    /// Check if labels contain a key
    pub fn contains_key(&self, key: &str) -> bool {
        self.labels.contains_key(key)
    }
    
    /// Get all labels
    pub fn all(&self) -> &HashMap<String, String> {
        &self.labels
    }
}

// ==================== Validation Metrics ====================

/// Metrics specific to validation operations
#[derive(Debug, Clone)]
pub struct ValidationMetrics {
    /// Total validations performed
    pub total_validations: u64,
    /// Successful validations
    pub successful_validations: u64,
    /// Failed validations
    pub failed_validations: u64,
    /// Validation duration histogram
    pub validation_duration: HistogramData,
    /// Error counts by error code
    pub error_counts: HashMap<ErrorCode, u64>,
    /// Validator usage counts
    pub validator_usage: HashMap<ValidatorId, u64>,
    /// Complexity distribution
    pub complexity_distribution: HashMap<ValidationComplexity, u64>,
    /// Cache performance metrics
    pub cache_metrics: CacheMetrics,
}

impl Default for ValidationMetrics {
    fn default() -> Self {
        Self {
            total_validations: 0,
            successful_validations: 0,
            failed_validations: 0,
            validation_duration: HistogramData::new(vec![0.001, 0.01, 0.1, 1.0, 10.0]),
            error_counts: HashMap::new(),
            validator_usage: HashMap::new(),
            complexity_distribution: HashMap::new(),
            cache_metrics: CacheMetrics::default(),
        }
    }
}

impl ValidationMetrics {
    /// Record a validation operation
    pub fn record_validation(
        &mut self,
        success: bool,
        duration: Duration,
        validator_id: &ValidatorId,
        complexity: ValidationComplexity,
    ) {
        self.total_validations += 1;
        
        if success {
            self.successful_validations += 1;
        } else {
            self.failed_validations += 1;
        }
        
        // Record duration
        self.validation_duration.record(duration.as_secs_f64());
        
        // Record validator usage
        *self.validator_usage.entry(validator_id.clone()).or_insert(0) += 1;
        
        // Record complexity distribution
        *self.complexity_distribution.entry(complexity).or_insert(0) += 1;
    }
    
    /// Record a validation error
    pub fn record_error(&mut self, error_code: ErrorCode) {
        *self.error_counts.entry(error_code).or_insert(0) += 1;
    }
    
    /// Get success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_validations == 0 {
            0.0
        } else {
            (self.successful_validations as f64 / self.total_validations as f64) * 100.0
        }
    }
    
    /// Get failure rate
    pub fn failure_rate(&self) -> f64 {
        if self.total_validations == 0 {
            0.0
        } else {
            (self.failed_validations as f64 / self.total_validations as f64) * 100.0
        }
    }
    
    /// Get average validation duration
    pub fn average_duration(&self) -> Duration {
        if self.validation_duration.count == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(self.validation_duration.sum / self.validation_duration.count as f64)
        }
    }
    
    /// Get p95 validation duration
    pub fn p95_duration(&self) -> Duration {
        self.validation_duration.percentile(0.95)
    }
    
    /// Get p99 validation duration
    pub fn p99_duration(&self) -> Duration {
        self.validation_duration.percentile(0.99)
    }
}

// ==================== Cache Metrics ====================

/// Metrics for cache performance
#[derive(Debug, Clone, Default)]
pub struct CacheMetrics {
    /// Cache hits
    pub hits: u64,
    /// Cache misses
    pub misses: u64,
    /// Cache evictions
    pub evictions: u64,
    /// Cache size
    pub size: usize,
    /// Cache capacity
    pub capacity: usize,
}

impl CacheMetrics {
    /// Record a cache hit
    pub fn record_hit(&mut self) {
        self.hits += 1;
    }
    
    /// Record a cache miss
    pub fn record_miss(&mut self) {
        self.misses += 1;
    }
    
    /// Record a cache eviction
    pub fn record_eviction(&mut self) {
        self.evictions += 1;
    }
    
    /// Update cache size
    pub fn update_size(&mut self, size: usize, capacity: usize) {
        self.size = size;
        self.capacity = capacity;
    }
    
    /// Get hit rate
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
    
    /// Get miss rate
    pub fn miss_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.misses as f64 / total as f64) * 100.0
        }
    }
    
    /// Get cache utilization
    pub fn utilization(&self) -> f64 {
        if self.capacity == 0 {
            0.0
        } else {
            (self.size as f64 / self.capacity as f64) * 100.0
        }
    }
}

// ==================== Histogram Implementation ====================

impl HistogramData {
    /// Create new histogram with bucket boundaries
    pub fn new(buckets: Vec<f64>) -> Self {
        let bucket_count = buckets.len();
        Self {
            buckets,
            bucket_counts: vec![0; bucket_count],
            sum: 0.0,
            count: 0,
        }
    }
    
    /// Record a value in the histogram
    pub fn record(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;
        
        // Find appropriate bucket
        for (i, &bucket) in self.buckets.iter().enumerate() {
            if value <= bucket {
                self.bucket_counts[i] += 1;
                break;
            }
        }
    }
    
    /// Get percentile value
    pub fn percentile(&self, p: f64) -> Duration {
        if self.count == 0 {
            return Duration::ZERO;
        }
        
        let target_count = (self.count as f64 * p) as u64;
        let mut current_count = 0;
        
        for (i, &count) in self.bucket_counts.iter().enumerate() {
            current_count += count;
            if current_count >= target_count {
                return Duration::from_secs_f64(self.buckets[i]);
            }
        }
        
        // If we get here, return the highest bucket
        Duration::from_secs_f64(self.buckets.last().copied().unwrap_or(0.0))
    }
    
    /// Get bucket boundaries
    pub fn buckets(&self) -> &[f64] {
        &self.buckets
    }
    
    /// Get bucket counts
    pub fn bucket_counts(&self) -> &[u64] {
        &self.bucket_counts
    }
    
    /// Get total sum
    pub fn sum(&self) -> f64 {
        self.sum
    }
    
    /// Get total count
    pub fn count(&self) -> u64 {
        self.count
    }
    
    /// Get mean value
    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum / self.count as f64
        }
    }
}

// ==================== Summary Implementation ====================

impl SummaryData {
    /// Create new summary with quantiles
    pub fn new(quantiles: Vec<f64>) -> Self {
        Self {
            quantiles,
            quantile_values: vec![0.0; quantiles.len()],
            sum: 0.0,
            count: 0,
        }
    }
    
    /// Record a value in the summary
    pub fn record(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;
        
        // Update quantile values (simplified implementation)
        // In a real implementation, you'd use a more sophisticated algorithm
        if self.count == 1 {
            for i in 0..self.quantiles.len() {
                self.quantile_values[i] = value;
            }
        }
    }
    
    /// Get value at specific quantile
    pub fn quantile(&self, p: f64) -> Option<f64> {
        for (i, &quantile) in self.quantiles.iter().enumerate() {
            if (quantile - p).abs() < f64::EPSILON {
                return Some(self.quantile_values[i]);
            }
        }
        None
    }
    
    /// Get all quantiles
    pub fn quantiles(&self) -> &[f64] {
        &self.quantiles
    }
    
    /// Get quantile values
    pub fn quantile_values(&self) -> &[f64] {
        &self.quantile_values
    }
    
    /// Get total sum
    pub fn sum(&self) -> f64 {
        self.sum
    }
    
    /// Get total count
    pub fn count(&self) -> u64 {
        self.count
    }
    
    /// Get mean value
    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum / self.count as f64
        }
    }
}

// ==================== Metrics Registry ====================

/// Central registry for all metrics
#[derive(Debug)]
pub struct MetricsRegistry {
    /// Validation metrics
    validation: Arc<RwLock<ValidationMetrics>>,
    /// System metrics
    system: Arc<RwLock<SystemMetrics>>,
    /// Custom metrics
    custom: Arc<RwLock<HashMap<String, MetricValue>>>,
    /// Metric definitions
    definitions: Arc<RwLock<HashMap<String, MetricDefinition>>>,
}

impl MetricsRegistry {
    /// Create new metrics registry
    pub fn new() -> Self {
        info!("Creating metrics registry");
        
        Self {
            validation: Arc::new(RwLock::new(ValidationMetrics::default())),
            system: Arc::new(RwLock::new(SystemMetrics::default())),
            custom: Arc::new(RwLock::new(HashMap::new())),
            definitions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Get validation metrics
    pub async fn validation(&self) -> ValidationMetrics {
        let metrics = self.validation.read().await;
        metrics.clone()
    }
    
    /// Get system metrics
    pub async fn system(&self) -> SystemMetrics {
        let metrics = self.system.read().await;
        metrics.clone()
    }
    
    /// Record a validation operation
    pub async fn record_validation(
        &self,
        success: bool,
        duration: Duration,
        validator_id: &ValidatorId,
        complexity: ValidationComplexity,
    ) {
        let mut metrics = self.validation.write().await;
        metrics.record_validation(success, duration, validator_id, complexity);
    }
    
    /// Record a validation error
    pub async fn record_error(&self, error_code: ErrorCode) {
        let mut metrics = self.validation.write().await;
        metrics.record_error(error_code);
    }
    
    /// Update cache metrics
    pub async fn update_cache_metrics(&self, cache_metrics: CacheMetrics) {
        let mut metrics = self.validation.write().await;
        metrics.cache_metrics = cache_metrics;
    }
    
    /// Update system metrics
    pub async fn update_system_metrics(&self) {
        let mut metrics = self.system.write().await;
        metrics.update();
    }
    
    /// Set custom metric
    pub async fn set_custom_metric(&self, name: String, value: MetricValue) {
        let mut metrics = self.custom.write().await;
        metrics.insert(name, value);
    }
    
    /// Get custom metric
    pub async fn get_custom_metric(&self, name: &str) -> Option<MetricValue> {
        let metrics = self.custom.read().await;
        metrics.get(name).cloned()
    }
    
    /// Define a new metric
    pub async fn define_metric(&self, definition: MetricDefinition) {
        let mut definitions = self.definitions.write().await;
        definitions.insert(definition.name.clone(), definition);
    }
    
    /// Get metric definition
    pub async fn get_metric_definition(&self, name: &str) -> Option<MetricDefinition> {
        let definitions = self.definitions.read().await;
        definitions.get(name).cloned()
    }
    
    /// Get all metrics as a combined view
    pub async fn all_metrics(&self) -> AllMetrics {
        let validation = self.validation.read().await;
        let system = self.system.read().await;
        let custom = self.custom.read().await;
        
        AllMetrics {
            validation: validation.clone(),
            system: system.clone(),
            custom: custom.clone(),
            timestamp: chrono::Utc::now(),
        }
    }
    
    /// Reset all metrics
    pub async fn reset(&self) {
        info!("Resetting all metrics");
        
        {
            let mut validation = self.validation.write().await;
            *validation = ValidationMetrics::default();
        }
        
        {
            let mut system = self.system.write().await;
            *system = SystemMetrics::default();
        }
        
        {
            let mut custom = self.custom.write().await;
            custom.clear();
        }
        
        info!("All metrics reset");
    }
}

// ==================== System Metrics ====================

/// System-level metrics
#[derive(Debug, Clone, Default)]
pub struct SystemMetrics {
    /// Memory usage in bytes
    pub memory_usage: u64,
    /// CPU usage percentage
    pub cpu_usage: f64,
    /// Thread count
    pub thread_count: usize,
    /// Uptime in seconds
    pub uptime: u64,
    /// Last update timestamp
    pub last_update: chrono::DateTime<chrono::Utc>,
}

impl SystemMetrics {
    /// Update system metrics
    pub fn update(&mut self) {
        // In a real implementation, you'd collect actual system metrics
        // For now, we'll use placeholder values
        self.memory_usage = 1024 * 1024 * 100; // 100 MB
        self.cpu_usage = 25.0; // 25%
        self.thread_count = 8;
        self.uptime = 3600; // 1 hour
        self.last_update = chrono::Utc::now();
    }
}

// ==================== All Metrics ====================

/// Combined view of all metrics
#[derive(Debug, Clone)]
pub struct AllMetrics {
    /// Validation metrics
    pub validation: ValidationMetrics,
    /// System metrics
    pub system: SystemMetrics,
    /// Custom metrics
    pub custom: HashMap<String, MetricValue>,
    /// Timestamp when metrics were collected
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ==================== Metrics Builder ====================

/// Builder for creating metrics registries
#[derive(Debug)]
pub struct MetricsBuilder {
    /// Whether to enable validation metrics
    enable_validation: bool,
    /// Whether to enable system metrics
    enable_system: bool,
    /// Whether to enable custom metrics
    enable_custom: bool,
}

impl MetricsBuilder {
    /// Create new metrics builder
    pub fn new() -> Self {
        Self {
            enable_validation: true,
            enable_system: true,
            enable_custom: true,
        }
    }
    
    /// Disable validation metrics
    pub fn without_validation(mut self) -> Self {
        self.enable_validation = false;
        self
    }
    
    /// Disable system metrics
    pub fn without_system(mut self) -> Self {
        self.enable_system = false;
        self
    }
    
    /// Disable custom metrics
    pub fn without_custom(mut self) -> Self {
        self.enable_custom = false;
        self
    }
    
    /// Build the metrics registry
    pub fn build(self) -> MetricsRegistry {
        MetricsRegistry::new()
    }
}

impl Default for MetricsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Re-exports ====================

pub use MetricsRegistry as Registry;
pub use MetricsBuilder as Builder;
pub use ValidationMetrics as Validation;
pub use CacheMetrics as Cache;
pub use SystemMetrics as System;
pub use AllMetrics as All;
pub use MetricValue as Value;
pub use MetricDefinition as Definition;
pub use MetricType as Type;
pub use MetricLabels as Labels;
pub use HistogramData as Histogram;
pub use SummaryData as Summary;
