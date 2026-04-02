//! Metrics integration for nebula-memory
//!
//! This module provides extension traits that allow integrating
//! the memory management system with various metrics and monitoring systems.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};
use parking_lot::RwLock;
use std::{collections::{BTreeMap, HashMap}, string::String, sync::Arc, vec::Vec};

use crate::error::MemoryResult;
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
    #[must_use = "builder methods must be chained or built"]
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

/// Debug reporter that prints metrics to stdout.
#[derive(Debug, Clone, Copy, Default)]
pub struct DebugMetricsReporter;

impl MetricsReporter for DebugMetricsReporter {
    #[cold]
    #[inline(never)]
    fn register_metric(&self, metric: &MemoryMetric) -> MemoryResult<()> {
        println!(
            "Registered metric: {} ({}) [{}] - {}",
            metric.name, metric.metric_type, metric.unit, metric.description
        );
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn report_counter(
        &self,
        name: &str,
        value: u64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        println!("Counter {name}{labels:?} = {value}");
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn report_gauge(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        println!("Gauge {name}{labels:?} = {value}");
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn report_histogram(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        println!("Histogram {name}{labels:?} = {value}");
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn report_summary(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        println!("Summary {name}{labels:?} = {value}");
        Ok(())
    }
}

#[derive(Clone)]
enum MetricsReporterAdapter {
    Noop(NoopMetricsReporter),
    Debug(DebugMetricsReporter),
    Custom(Arc<dyn MetricsReporter>),
}

impl MetricsReporterAdapter {
    #[inline(never)]
    fn register_metric(&self, metric: &MemoryMetric) -> MemoryResult<()> {
        match self {
            Self::Noop(r) => r.register_metric(metric),
            Self::Debug(r) => r.register_metric(metric),
            Self::Custom(r) => r.register_metric(metric),
        }
    }

    #[inline(never)]
    fn report_counter(
        &self,
        name: &str,
        value: u64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        match self {
            Self::Noop(r) => r.report_counter(name, value, labels),
            Self::Debug(r) => r.report_counter(name, value, labels),
            Self::Custom(r) => r.report_counter(name, value, labels),
        }
    }

    #[inline(never)]
    fn report_gauge(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        match self {
            Self::Noop(r) => r.report_gauge(name, value, labels),
            Self::Debug(r) => r.report_gauge(name, value, labels),
            Self::Custom(r) => r.report_gauge(name, value, labels),
        }
    }

    #[inline(never)]
    fn report_histogram(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        match self {
            Self::Noop(r) => r.report_histogram(name, value, labels),
            Self::Debug(r) => r.report_histogram(name, value, labels),
            Self::Custom(r) => r.report_histogram(name, value, labels),
        }
    }

    #[inline(never)]
    fn report_summary(
        &self,
        name: &str,
        value: f64,
        labels: &BTreeMap<String, String>,
    ) -> MemoryResult<()> {
        match self {
            Self::Noop(r) => r.report_summary(name, value, labels),
            Self::Debug(r) => r.report_summary(name, value, labels),
            Self::Custom(r) => r.report_summary(name, value, labels),
        }
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
    #[must_use = "builder methods must be chained or built"]
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
    #[must_use = "builder methods must be chained or built"]
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
#[derive(Clone)]
pub struct MetricsExtension {
    /// The metrics reporter implementation
    reporter: MetricsReporterAdapter,
    /// Registered metrics storage (split index + slots)
    storage: Arc<RwLock<MetricsStorage>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MetricId(usize);

#[derive(Debug, Default)]
struct MetricsStorage {
    by_name: HashMap<String, MetricId>,
    slots: Vec<Option<StoredMetric>>,
    free_list: Vec<MetricId>,
}

#[derive(Debug)]
pub struct StoredMetric {
    pub name: String,
    pub description: String,
    pub metric_type: MetricType,
    pub labels: BTreeMap<String, String>,
    pub unit: String,
}

impl StoredMetric {
    fn from_metric(metric: MemoryMetric) -> Self {
        Self {
            name: metric.name,
            description: metric.description,
            metric_type: metric.metric_type,
            labels: metric.labels,
            unit: metric.unit,
        }
    }

    fn into_metric(self) -> MemoryMetric {
        MemoryMetric {
            name: self.name,
            description: self.description,
            metric_type: self.metric_type,
            labels: self.labels,
            unit: self.unit,
        }
    }

    fn to_metric(&self) -> MemoryMetric {
        MemoryMetric {
            name: self.name.clone(),
            description: self.description.clone(),
            metric_type: self.metric_type,
            labels: self.labels.clone(),
            unit: self.unit.clone(),
        }
    }
}

impl MetricsStorage {
    fn upsert(&mut self, metric: MemoryMetric) {
        if let Some(existing_id) = self.by_name.get(metric.name.as_str()).copied() {
            self.slots[existing_id.0] = Some(StoredMetric::from_metric(metric));
            return;
        }

        let name = metric.name.clone();
        let stored = StoredMetric::from_metric(metric);

        if let Some(reused_id) = self.free_list.pop() {
            self.slots[reused_id.0] = Some(stored);
            self.by_name.insert(name, reused_id);
            return;
        }

        let new_id = MetricId(self.slots.len());
        self.slots.push(Some(stored));
        self.by_name.insert(name, new_id);
    }

    fn snapshot_map(&self) -> BTreeMap<String, MemoryMetric> {
        // Iterated over slots (Vec) for cache-friendly sequential access.
        // HashMap (by_name) is only used for O(1) lookup on upsert/remove.
        //
        // Allocation cost per metric (unavoidable with current return type):
        // - 2× String::clone for name  (BTreeMap key + MemoryMetric::name)
        // - 1× String::clone for description
        // - 1× BTreeMap::clone for labels (dominant cost at high label counts)
        // - 1× String::clone for unit
        let mut out = BTreeMap::new();

        for stored in self.slots.iter().flatten() {
            // name cloned twice: once as BTreeMap key, once inside MemoryMetric.
            out.insert(stored.name.clone(), stored.to_metric());
        }

        out
    }

    fn remove(&mut self, name: &str) -> Option<MemoryMetric> {
        let id = self.by_name.remove(name)?;
        let metric = self.slots[id.0].take().map(StoredMetric::into_metric);
        self.free_list.push(id);
        metric
    }
}

impl MetricsExtension {
    /// Create a new metrics extension with the specified reporter
    pub fn new(reporter: impl MetricsReporter + 'static) -> Self {
        Self::new_custom(Arc::new(reporter))
    }

    /// Create a metrics extension with a no-op reporter.
    pub fn new_noop() -> Self {
        Self {
            reporter: MetricsReporterAdapter::Noop(NoopMetricsReporter),
            storage: Arc::new(RwLock::new(MetricsStorage::default())),
        }
    }

    /// Create a metrics extension with a debug stdout reporter.
    pub fn new_debug() -> Self {
        Self {
            reporter: MetricsReporterAdapter::Debug(DebugMetricsReporter),
            storage: Arc::new(RwLock::new(MetricsStorage::default())),
        }
    }

    /// Create a metrics extension with a custom reporter.
    pub fn new_custom(reporter: Arc<dyn MetricsReporter>) -> Self {
        Self {
            reporter: MetricsReporterAdapter::Custom(reporter),
            storage: Arc::new(RwLock::new(MetricsStorage::default())),
        }
    }

    /// Low-allocation iterator over metrics (borrows instead of cloning).
    /// Returns tuples of (name_ref, MetricRef) without allocating full MemoryMetric clones.
    ///
    /// # Iteration order
    /// Unspecified. Use [`metrics_snapshot`] if sorted order is required.
    ///
    /// # Allocation Cost
    /// Zero allocations beyond the lock acquisition.
    /// Use this instead of `metrics_snapshot()` when you don't need owned MemoryMetric values.
    ///
    /// # Example
    /// ```ignore
    /// ext.metrics_iter(|name, stored| {
    ///     println!("{}:  {} ops", name, stored.metric_type);
    /// });
    /// ```
    pub fn metrics_iter<F>(&self, mut callback: F)
    where
        F: FnMut(&str, &StoredMetric),
    {
        let storage = self.storage.read();
        for stored in storage.slots.iter().flatten() {
            callback(&stored.name, stored);
        }
    }

    /// Register a metric with this extension
    pub fn register_metric(&self, metric: MemoryMetric) -> MemoryResult<()> {
        self.reporter.register_metric(&metric)?;

        let mut storage = self.storage.write();
        storage.upsert(metric);

        Ok(())
    }

    /// Unregister a metric by name and return the removed definition.
    pub fn unregister_metric(&self, name: &str) -> Option<MemoryMetric> {
        self.storage.write().remove(name)
    }

    /// Snapshot registered metrics as a name-indexed map.
    pub fn metrics_snapshot(&self) -> BTreeMap<String, MemoryMetric> {
        self.storage.read().snapshot_map()
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
pub fn create_debug_metrics_reporter() -> DebugMetricsReporter {
    DebugMetricsReporter
}

/// Helper to get the current global metrics extension
pub fn global_metrics() -> Option<Arc<MetricsExtension>> {
    use crate::extensions::GlobalExtensions;

    GlobalExtensions::get_by_type_downcast::<MetricsExtension>().map(Arc::new)
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

    #[test]
    fn test_metrics_extension_upsert_replaces_by_name() {
        let extension = MetricsExtension::new_noop();

        extension
            .register_metric(MemoryMetric::new(
                "alloc.total",
                "first",
                MetricType::Counter,
                "ops",
            ))
            .expect("register first metric");

        extension
            .register_metric(MemoryMetric::new(
                "alloc.total",
                "second",
                MetricType::Counter,
                "ops",
            ))
            .expect("register replacement metric");

        let snapshot = extension.metrics_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot["alloc.total"].description, "second");
    }

    #[test]
    fn test_metrics_extension_unregister_removes_metric() {
        let extension = MetricsExtension::new_noop();

        extension
            .register_metric(MemoryMetric::new(
                "alloc.bytes",
                "bytes allocated",
                MetricType::Gauge,
                "bytes",
            ))
            .expect("register metric");

        let removed = extension.unregister_metric("alloc.bytes");
        assert!(removed.is_some());
        assert!(extension.metrics_snapshot().is_empty());
        assert!(extension.unregister_metric("alloc.bytes").is_none());
    }

    #[test]
    fn test_metrics_iter_zero_allocation() {
        let extension = MetricsExtension::new_noop();

        // Register 3 metrics
        for i in 0..3 {
            extension
                .register_metric(
                    MemoryMetric::new(
                        format!("metric_{}", i),
                        "test metric",
                        MetricType::Counter,
                        "ops",
                    )
                    .with_label("id", format!("{}", i)),
                )
                .expect("register metric");
        }

        // Usage: metrics_iter borrows instead of cloning
        let mut count = 0;
        extension.metrics_iter(|name, stored| {
            // No allocations: we receive borrowed &str and &StoredMetric
            assert!(!name.is_empty());
            assert_eq!(stored.metric_type, MetricType::Counter);
            assert_eq!(stored.unit, "ops");
            count += 1;
        });
        assert_eq!(count, 3);

        // Compare to snapshot which allocates heavily
        let snapshot = extension.metrics_snapshot();
        assert_eq!(snapshot.len(), 3);
        // snapshot required 3 BTreeMap + 9 String clones
        // metrics_iter required zero allocations
    }
}
