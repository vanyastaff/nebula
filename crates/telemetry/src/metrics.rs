//! Metrics primitives and registry.
//!
//! Provides lightweight metric types (counter, gauge, histogram) and a
//! registry to create and retrieve them. The MVP implementation stores
//! values in-memory with atomics -- no external exporter needed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// An incrementing counter.
#[derive(Debug, Clone)]
pub struct Counter {
    value: Arc<AtomicU64>,
}

impl Counter {
    /// Create a new counter starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment by a given amount.
    pub fn inc_by(&self, n: u64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// A gauge that can go up and down.
#[derive(Debug, Clone)]
pub struct Gauge {
    value: Arc<AtomicI64>,
}

impl Gauge {
    /// Create a new gauge starting at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Increment by one.
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement by one.
    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }

    /// Set to a specific value.
    pub fn set(&self, v: i64) {
        self.value.store(v, Ordering::Relaxed);
    }

    /// Current value.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

/// A simple histogram that records observations.
///
/// Stores all observations in memory. Suitable for development and
/// testing but not for production with millions of data points.
#[derive(Debug, Clone)]
pub struct Histogram {
    observations: Arc<RwLock<Vec<f64>>>,
}

impl Histogram {
    /// Create a new histogram.
    #[must_use]
    pub fn new() -> Self {
        Self {
            observations: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Record an observation.
    pub fn observe(&self, value: f64) {
        self.observations
            .write()
            .expect("histogram lock poisoned")
            .push(value);
    }

    /// Number of observations recorded.
    #[must_use]
    pub fn count(&self) -> usize {
        self.observations
            .read()
            .expect("histogram lock poisoned")
            .len()
    }

    /// Sum of all observations.
    #[must_use]
    pub fn sum(&self) -> f64 {
        self.observations
            .read()
            .expect("histogram lock poisoned")
            .iter()
            .sum()
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

/// Registry for creating and retrieving named metrics.
///
/// # Examples
///
/// ```
/// use nebula_telemetry::metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let counter = registry.counter("executions_total");
/// counter.inc();
/// assert_eq!(counter.get(), 1);
///
/// // Retrieving the same name returns the same metric.
/// let same = registry.counter("executions_total");
/// assert_eq!(same.get(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    counters: Arc<RwLock<HashMap<String, Counter>>>,
    gauges: Arc<RwLock<HashMap<String, Gauge>>>,
    histograms: Arc<RwLock<HashMap<String, Histogram>>>,
}

impl MetricsRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            counters: Arc::new(RwLock::new(HashMap::new())),
            gauges: Arc::new(RwLock::new(HashMap::new())),
            histograms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get or create a counter by name.
    pub fn counter(&self, name: &str) -> Counter {
        let mut map = self.counters.write().expect("counter lock poisoned");
        map.entry(name.to_owned()).or_default().clone()
    }

    /// Get or create a gauge by name.
    pub fn gauge(&self, name: &str) -> Gauge {
        let mut map = self.gauges.write().expect("gauge lock poisoned");
        map.entry(name.to_owned()).or_default().clone()
    }

    /// Get or create a histogram by name.
    pub fn histogram(&self, name: &str) -> Histogram {
        let mut map = self.histograms.write().expect("histogram lock poisoned");
        map.entry(name.to_owned()).or_default().clone()
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// A no-op metrics registry that discards all observations.
///
/// Useful for testing and contexts where metrics are not needed.
#[derive(Debug, Clone, Copy)]
pub struct NoopMetricsRegistry;

impl NoopMetricsRegistry {
    /// Create a noop registry.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopMetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_starts_at_zero() {
        let c = Counter::new();
        assert_eq!(c.get(), 0);
    }

    #[test]
    fn counter_increments() {
        let c = Counter::new();
        c.inc();
        c.inc_by(5);
        assert_eq!(c.get(), 6);
    }

    #[test]
    fn gauge_up_and_down() {
        let g = Gauge::new();
        g.inc();
        g.inc();
        g.dec();
        assert_eq!(g.get(), 1);
        g.set(42);
        assert_eq!(g.get(), 42);
    }

    #[test]
    fn histogram_records_observations() {
        let h = Histogram::new();
        h.observe(1.0);
        h.observe(2.5);
        h.observe(3.0);
        assert_eq!(h.count(), 3);
        assert!((h.sum() - 6.5).abs() < f64::EPSILON);
    }

    #[test]
    fn registry_returns_same_metric_for_same_name() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("requests");
        c1.inc();
        let c2 = reg.counter("requests");
        assert_eq!(c2.get(), 1);
    }

    #[test]
    fn registry_different_names_are_independent() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("a");
        let c2 = reg.counter("b");
        c1.inc();
        assert_eq!(c1.get(), 1);
        assert_eq!(c2.get(), 0);
    }
}
