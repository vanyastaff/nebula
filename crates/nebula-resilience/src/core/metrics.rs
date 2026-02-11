//! High-performance metrics collection and reporting with security hardening

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Maximum number of unique metrics to prevent memory exhaustion attacks
const MAX_METRICS_COUNT: usize = 10_000;

/// Maximum metric name length to prevent `DoS` attacks
const MAX_METRIC_NAME_LENGTH: usize = 256;

/// Metrics collector
pub struct MetricsCollector {
    metrics: Arc<RwLock<HashMap<String, Metric>>>,
    enabled: bool,
}

impl MetricsCollector {
    /// Create new metrics collector
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
            enabled,
        }
    }

    /// Record a value with security validation
    pub fn record(&self, name: impl Into<String>, value: f64) {
        if !self.enabled {
            return;
        }

        let name = name.into();

        // Security validation: check name length
        if name.len() > MAX_METRIC_NAME_LENGTH {
            return; // Silently drop suspicious metrics
        }

        // Security validation: check for NaN/Infinite values
        if !value.is_finite() {
            return;
        }

        let mut metrics = self.metrics.write();

        // Security validation: prevent memory exhaustion
        if metrics.len() >= MAX_METRICS_COUNT && !metrics.contains_key(&name) {
            return; // Drop new metrics if at capacity
        }

        metrics
            .entry(name)
            .or_insert_with(Metric::new)
            .record(value);
    }

    /// Increment counter
    pub fn increment(&self, name: impl Into<String>) {
        self.record(name, 1.0);
    }

    /// Record duration
    pub fn record_duration(&self, name: impl Into<String>, duration: Duration) {
        self.record(name, duration.as_secs_f64() * 1000.0);
    }

    /// Start a timer
    pub fn start_timer(&self, name: impl Into<String>) -> MetricTimer {
        MetricTimer::new(name.into(), self.enabled)
    }

    /// Get metric snapshot
    #[must_use]
    pub fn snapshot(&self, name: &str) -> Option<MetricSnapshot> {
        let metrics = self.metrics.read();
        metrics.get(name).map(Metric::snapshot)
    }

    /// Get all metrics
    #[must_use]
    pub fn all_metrics(&self) -> HashMap<String, MetricSnapshot> {
        let metrics = self.metrics.read();
        metrics
            .iter()
            .map(|(k, v)| (k.clone(), v.snapshot()))
            .collect()
    }

    /// Clear all metrics
    pub fn clear(&self) {
        self.metrics.write().clear();
    }
}

/// Individual metric
pub(super) struct Metric {
    count: AtomicU64,
    sum: AtomicU64,
    min: AtomicU64,
    max: AtomicU64,
}

impl Metric {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0.0_f64.to_bits()),
            min: AtomicU64::new(f64::INFINITY.to_bits()),
            max: AtomicU64::new(0.0_f64.to_bits()),
        }
    }

    fn record(&self, value: f64) {
        self.count.fetch_add(1, Ordering::Relaxed);

        // CAS loop to atomically add the float value (not the bit pattern)
        let mut current = self.sum.load(Ordering::Relaxed);
        loop {
            let new = f64::from_bits(current) + value;
            match self.sum.compare_exchange_weak(
                current,
                new.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current = x,
            }
        }

        // Update min — compare actual float values, not bit patterns
        let mut current_min = self.min.load(Ordering::Relaxed);
        while value < f64::from_bits(current_min) {
            match self.min.compare_exchange_weak(
                current_min,
                value.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_min = x,
            }
        }

        // Update max — compare actual float values, not bit patterns
        let mut current_max = self.max.load(Ordering::Relaxed);
        while value > f64::from_bits(current_max) {
            match self.max.compare_exchange_weak(
                current_max,
                value.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
    }

    fn snapshot(&self) -> MetricSnapshot {
        let count = self.count.load(Ordering::Relaxed);
        let sum = f64::from_bits(self.sum.load(Ordering::Relaxed));
        let min = f64::from_bits(self.min.load(Ordering::Relaxed));
        let max = f64::from_bits(self.max.load(Ordering::Relaxed));

        MetricSnapshot {
            count,
            sum,
            min: if count > 0 { min } else { 0.0 },
            max: if count > 0 { max } else { 0.0 },
            avg: if count > 0 { sum / count as f64 } else { 0.0 },
        }
    }
}

/// Metric snapshot containing aggregated statistics
#[derive(Debug, Clone)]
pub struct MetricSnapshot {
    /// Number of recorded values
    pub count: u64,
    /// Sum of all recorded values
    pub sum: f64,
    /// Minimum recorded value
    pub min: f64,
    /// Maximum recorded value
    pub max: f64,
    /// Average of all recorded values
    pub avg: f64,
}

/// Timer for recording durations
pub struct MetricTimer {
    name: String,
    start: Option<Instant>,
    /// Whether metrics collection is enabled
    #[allow(dead_code)]
    enabled: bool,
}

impl MetricTimer {
    fn new(name: String, enabled: bool) -> Self {
        Self {
            name,
            start: if enabled { Some(Instant::now()) } else { None },
            enabled,
        }
    }

    /// Stop timer and record duration
    pub fn stop(self, collector: &MetricsCollector) {
        if let Some(start) = self.start {
            collector.record_duration(&self.name, start.elapsed());
        }
    }
}

/// Global metrics instance for application-wide metrics collection
///
/// This is currently unused but reserved for future integration with
/// observability systems like Prometheus/OpenTelemetry.
#[allow(dead_code)]
static GLOBAL_METRICS: std::sync::LazyLock<MetricsCollector> =
    std::sync::LazyLock::new(|| MetricsCollector::new(true));

/// Get global metrics collector
///
/// Reserved for future metrics integration. Currently unused.
#[allow(dead_code)]
pub(super) fn global_metrics() -> &'static MetricsCollector {
    &GLOBAL_METRICS
}

/// Metric kinds for categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    /// Counter metric that only increases
    Counter,
    /// Gauge metric that can increase or decrease
    Gauge,
    /// Histogram metric for distribution of values
    Histogram,
    /// Timer metric for measuring durations
    Timer,
}

/// Extended metrics with metadata
pub struct Metrics {
    collector: MetricsCollector,
    prefix: String,
    tags: HashMap<String, String>,
}

impl Metrics {
    /// Create new metrics instance
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            collector: MetricsCollector::new(true),
            prefix: prefix.into(),
            tags: HashMap::new(),
        }
    }

    /// Add tag
    #[must_use = "builder methods must be chained or built"]
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Format metric name with prefix
    fn format_name(&self, name: &str) -> String {
        if self.prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.prefix, name)
        }
    }

    /// Record metric
    pub fn record(&self, name: &str, value: f64) {
        self.collector.record(self.format_name(name), value);
    }

    /// Increment counter
    pub fn increment(&self, name: &str) {
        self.collector.increment(self.format_name(name));
    }

    /// Start timer
    #[must_use]
    pub fn timer(&self, name: &str) -> MetricTimer {
        self.collector.start_timer(self.format_name(name))
    }
}
