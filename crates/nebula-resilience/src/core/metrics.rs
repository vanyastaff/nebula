//! Metrics collection and reporting

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use parking_lot::RwLock;

/// Metrics collector
pub struct MetricsCollector {
    metrics: Arc<RwLock<HashMap<String, Metric>>>,
    enabled: bool,
}

impl MetricsCollector {
    /// Create new metrics collector
    pub fn new(enabled: bool) -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
            enabled,
        }
    }

    /// Record a value
    pub fn record(&self, name: impl Into<String>, value: f64) {
        if !self.enabled {
            return;
        }

        let name = name.into();
        let mut metrics = self.metrics.write();

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
    pub fn snapshot(&self, name: &str) -> Option<MetricSnapshot> {
        let metrics = self.metrics.read();
        metrics.get(name).map(|m| m.snapshot())
    }

    /// Get all metrics
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
pub struct Metric {
    count: AtomicU64,
    sum: AtomicU64,
    min: AtomicU64,
    max: AtomicU64,
}

impl Metric {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
            min: AtomicU64::new(u64::MAX),
            max: AtomicU64::new(0),
        }
    }

    fn record(&self, value: f64) {
        let value_bits = value.to_bits();

        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value_bits, Ordering::Relaxed);

        // Update min
        let mut current_min = self.min.load(Ordering::Relaxed);
        while value_bits < current_min {
            match self.min.compare_exchange_weak(
                current_min,
                value_bits,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(x) => current_min = x,
            }
        }

        // Update max
        let mut current_max = self.max.load(Ordering::Relaxed);
        while value_bits > current_max {
            match self.max.compare_exchange_weak(
                current_max,
                value_bits,
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

/// Metric snapshot
#[derive(Debug, Clone)]
pub struct MetricSnapshot {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
}

/// Timer for recording durations
pub struct MetricTimer {
    name: String,
    start: Option<Instant>,
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

/// Global metrics instance
static GLOBAL_METRICS: once_cell::sync::Lazy<MetricsCollector> =
    once_cell::sync::Lazy::new(|| MetricsCollector::new(true));

/// Get global metrics collector
pub fn global_metrics() -> &'static MetricsCollector {
    &GLOBAL_METRICS
}

/// Metric kinds for categorization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
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
    pub fn timer(&self, name: &str) -> MetricTimer {
        self.collector.start_timer(self.format_name(name))
    }
}