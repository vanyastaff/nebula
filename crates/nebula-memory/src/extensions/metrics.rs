//! Metrics integration for nebula-memory
//!
//! This module provides extension traits that allow integrating
//! the memory management system with various metrics and monitoring systems.

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "std")]
use std::{boxed::Box, collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use crate::core::error::MemoryResult;
use crate::extensions::MemoryExtension;

/// Types of memory metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricType {
    /// Counter that can only increase (e.g., total allocations)
    Counter,
    /// Gauge that can go up and down (e.g., current memory usage)
    Gauge,
    /// Histogram for distributions (e.g., allocation size distribution)
    Histogram,
    /// Summary for percentiles (e.g., allocation latency)
    Summary,
}

impl fmt::Display for MetricType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Counter => write!(f, "counter"),
            Self::Gauge => write!(f, "gauge"),
            Self::Histogram => write!(f, "histogram"),
            Self::Summary => write!(f, "summary"),
        }
    }
}

/// A memory metric with metadata
#[derive(Debug, Clone)]
pub struct MemoryMetric {
    /// Unique name of the metric
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Type of metric
    pub metric_type: MetricType,
    /// Labels/tags for this metric
    pub labels: BTreeMap<String, String>,
    /// Unit of measurement (bytes, operations, seconds, etc.)
    pub unit: String,
}

impl MemoryMetric {
    /// Create a new memory metric
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        metric_type: MetricType,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            metric_type,
            labels: BTreeMap::new(),
            unit: unit.into(),
        }
    }

    /// Add a label to this metric
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }
}

/// Trait for reporting memory metrics
pub trait MetricsReporter: Send + Sync {
    /// Register a metric with the metrics system
    fn register_metric(&self, metric: &MemoryMetric) -> MemoryResult<()>;

    /// Report a counter value
    fn report_counter(
        &self,
        name: &str,
        value: u64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()>;

    /// Report a gauge value
    fn report_gauge(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()>;

    /// Report a histogram observation
    fn report_histogram(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()>;

    /// Report a summary observation
    fn report_summary(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()>;
}

/// No-op metrics reporter that discards all metrics
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopMetricsReporter;

impl MetricsReporter for NoopMetricsReporter {
    fn register_metric(&self, _metric: &MemoryMetric) -> MemoryResult<()> {
        Ok(())
    }

    fn report_counter(
        &self,
        _name: &str,
        _value: u64,
        _labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        Ok(())
    }

    fn report_gauge(
        &self,
        _name: &str,
        _value: f64,
        _labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        Ok(())
    }

    fn report_histogram(
        &self,
        _name: &str,
        _value: f64,
        _labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        Ok(())
    }

    fn report_summary(
        &self,
        _name: &str,
        _value: f64,
        _labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        Ok(())
    }
}

/// A simple counter metric that can be incremented
pub struct Counter {
    metric: MemoryMetric,
    value: AtomicU64,
}

impl Counter {
    /// Create a new counter
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            metric: MemoryMetric::new(name, description, MetricType::Counter, unit),
            value: AtomicU64::new(0),
        }
    }

    /// Add a label to this counter
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metric = self.metric.with_label(key, value);
        self
    }

    /// Increment the counter by the specified amount
    pub fn inc(&self, value: u64) {
        self.value.fetch_add(value, Ordering::Relaxed);
    }

    /// Get the current value
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Get the metric definition
    pub fn metric(&self) -> &MemoryMetric {
        &self.metric
    }

    /// Report this counter to a metrics reporter
    pub fn report(&self, reporter: &dyn MetricsReporter) -> MemoryResult<()> {
        reporter.report_counter(&self.metric.name, self.value(), &self.metric.labels)
    }
}

/// A simple gauge metric that can go up and down
pub struct Gauge {
    metric: MemoryMetric,
    value: AtomicU64,
}

impl Gauge {
    /// Create a new gauge
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            metric: MemoryMetric::new(name, description, MetricType::Gauge, unit),
            value: AtomicU64::new(0),
        }
    }

    /// Add a label to this gauge
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metric = self.metric.with_label(key, value);
        self
    }

    /// Set the gauge to a specific value
    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
    }

    /// Increment the gauge by the specified amount
    pub fn inc(&self, value: u64) {
        self.value.fetch_add(value, Ordering::Relaxed);
    }

    /// Decrement the gauge by the specified amount
    pub fn dec(&self, value: u64) {
        self.value.fetch_sub(value, Ordering::Relaxed);
    }

    /// Get the current value
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Get the metric definition
    pub fn metric(&self) -> &MemoryMetric {
        &self.metric
    }

    /// Report this gauge to a metrics reporter
    pub fn report(&self, reporter: &dyn MetricsReporter) -> MemoryResult<()> {
        reporter.report_gauge(&self.metric.name, self.value() as f64, &self.metric.labels)
    }
}

/// Memory metrics extension
pub struct MetricsExtension {
    /// The metrics reporter implementation
    reporter: Box<dyn MetricsReporter>,
    /// Registered metrics
    metrics: BTreeMap<String, MemoryMetric>,
}

impl MetricsExtension {
    /// Create a new metrics extension with the specified reporter
    pub fn new(reporter: impl MetricsReporter + 'static) -> Self {
        Self { reporter: Box::new(reporter), metrics: BTreeMap::new() }
    }

    /// Register a metric with this extension
    pub fn register_metric(&mut self, metric: MemoryMetric) -> MemoryResult<()> {
        self.reporter.register_metric(&metric)?;
        self.metrics.insert(metric.name.clone(), metric);
        Ok(())
    }

    /// Get the current reporter
    pub fn reporter(&self) -> &dyn MetricsReporter {
        self.reporter.as_ref()
    }

    /// Report a counter value
    pub fn report_counter(
        &self,
        name: &str,
        value: u64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.reporter.report_counter(name, value, labels)
    }

    /// Report a gauge value
    pub fn report_gauge(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.reporter.report_gauge(name, value, labels)
    }

    /// Report a histogram observation
    pub fn report_histogram(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.reporter.report_histogram(name, value, labels)
    }

    /// Report a summary observation
    pub fn report_summary(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.reporter.report_summary(name, value, labels)
    }
}

impl MemoryExtension for MetricsExtension {
    fn name(&self) -> &str {
        "metrics"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn category(&self) -> &str {
        "metrics"
    }

    fn tags(&self) -> Vec<&str> {
        vec!["metrics", "monitoring"]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// Create a debug metrics reporter that prints metrics to stdout
#[cfg(feature = "std")]
pub fn create_debug_metrics_reporter() -> impl MetricsReporter {
    struct DebugMetricsReporter;

    impl MetricsReporter for DebugMetricsReporter {
        fn register_metric(&self, metric: &MemoryMetric) -> MemoryResult<()> {
            println!(
                "Registered metric: {} ({}) [{}] - {}",
                metric.name, metric.metric_type, metric.unit, metric.description
            );
            Ok(())
        }

        fn report_counter(
            &self,
            name: &str,
            value: u64,
            labels: &BTreeMap<String, String>,
        ) -> MemoryResult<()> {
            println!("Counter {}{:?} = {}", name, labels, value);
            Ok(())
        }

        fn report_gauge(
            &self,
            name: &str,
            value: f64,
            labels: &BTreeMap<String, String>,
        ) -> MemoryResult<()> {
            println!("Gauge {}{:?} = {}", name, labels, value);
            Ok(())
        }

        fn report_histogram(
            &self,
            name: &str,
            value: f64,
            labels: &BTreeMap<String, String>,
        ) -> MemoryResult<()> {
            println!("Histogram {}{:?} = {}", name, labels, value);
            Ok(())
        }

        fn report_summary(
            &self,
            name: &str,
            value: f64,
            labels: &BTreeMap<String, String>,
        ) -> MemoryResult<()> {
            println!("Summary {}{:?} = {}", name, labels, value);
            Ok(())
        }
    }

    DebugMetricsReporter
}

/// Helper to get the current global metrics extension
pub fn global_metrics() -> Option<Arc<MetricsExtension>> {
    use crate::extensions::GlobalExtensions;

    if let Some(ext) = GlobalExtensions::get("metrics") {
        if let Some(metrics_ext) = ext.as_any().downcast_ref::<MetricsExtension>() {
            // Создаем новую обертку для репортера с использованием Arc
            let reporter: Arc<dyn MetricsReporter + 'static> = Arc::new(NoopMetricsReporter);

            // Создаем новый экземпляр с новым репортером, который делегирует вызовы
            let reporter_wrapper = DelegatingReporter { inner: reporter };

            return Some(Arc::new(MetricsExtension {
                reporter: Box::new(reporter_wrapper),
                metrics: metrics_ext.metrics.clone(),
            }));
        }
    }
    None
}

/// Репортер метрик, который делегирует вызовы другому репортеру.
/// Эта структура использует Arc для хранения репортера вместо простой ссылки.
struct DelegatingReporter {
    inner: Arc<dyn MetricsReporter + 'static>,
}

impl MetricsReporter for DelegatingReporter {
    fn register_metric(&self, metric: &MemoryMetric) -> MemoryResult<()> {
        self.inner.register_metric(metric)
    }

    fn report_counter(
        &self,
        name: &str,
        value: u64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.inner.report_counter(name, value, labels)
    }

    fn report_gauge(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.inner.report_gauge(name, value, labels)
    }

    fn report_histogram(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.inner.report_histogram(name, value, labels)
    }

    fn report_summary(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        self.inner.report_summary(name, value, labels)
    }
}

/// Initialize the global metrics reporter
pub fn init_global_metrics(reporter: impl MetricsReporter + 'static) -> MemoryResult<()> {
    use crate::extensions::GlobalExtensions;

    let extension = MetricsExtension::new(reporter);
    GlobalExtensions::register(extension)
}

/// Report a counter metric through the global metrics reporter (if configured)
pub fn report_counter(
    name: &str,
    value: u64,
    labels: &BTreeMap<String, String>,
) -> MemoryResult<()> {
    if let Some(metrics) = global_metrics() {
        metrics.report_counter(name, value, labels)?;
    }
    Ok(())
}

/// Report a gauge metric through the global metrics reporter (if configured)
pub fn report_gauge(name: &str, value: f64, labels: &BTreeMap<String, String>) -> MemoryResult<()> {
    if let Some(metrics) = global_metrics() {
        metrics.report_gauge(name, value, labels)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter() {
        let counter =
            Counter::new("test_counter", "A test counter", "operations").with_label("test", "true");

        counter.inc(5);
        assert_eq!(counter.value(), 5);

        counter.inc(10);
        assert_eq!(counter.value(), 15);

        let metric = counter.metric();
        assert_eq!(metric.name, "test_counter");
        assert_eq!(metric.description, "A test counter");
        assert_eq!(metric.metric_type, MetricType::Counter);
        assert_eq!(metric.unit, "operations");
        assert_eq!(metric.labels.get("test"), Some(&"true".to_string()));
    }

    #[test]
    fn test_gauge() {
        let gauge = Gauge::new("test_gauge", "A test gauge", "bytes").with_label("test", "true");

        gauge.set(100);
        assert_eq!(gauge.value(), 100);

        gauge.inc(50);
        assert_eq!(gauge.value(), 150);

        gauge.dec(30);
        assert_eq!(gauge.value(), 120);

        let metric = gauge.metric();
        assert_eq!(metric.name, "test_gauge");
        assert_eq!(metric.description, "A test gauge");
        assert_eq!(metric.metric_type, MetricType::Gauge);
        assert_eq!(metric.unit, "bytes");
        assert_eq!(metric.labels.get("test"), Some(&"true".to_string()));
    }

    #[test]
    fn test_noop_metrics_reporter() {
        let reporter = NoopMetricsReporter;
        let counter = Counter::new("test", "test", "count");

        // These should not panic
        assert!(reporter.register_metric(counter.metric()).is_ok());
        assert!(counter.report(&reporter).is_ok());
    }
}
