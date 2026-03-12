//! Metrics primitives and registry.
//!
//! Provides lightweight metric types (counter, gauge, histogram) and a
//! registry to create and retrieve them. The MVP implementation stores
//! values in-memory with atomics -- no external exporter needed.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tracing::warn;

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
            .unwrap_or_else(|poisoned| {
                warn!("histogram lock was poisoned, recovering");
                poisoned.into_inner()
            })
            .push(value);
    }

    /// Number of observations recorded.
    #[must_use]
    pub fn count(&self) -> usize {
        self.observations
            .read()
            .unwrap_or_else(|poisoned| {
                warn!("histogram lock was poisoned, recovering");
                poisoned.into_inner()
            })
            .len()
    }

    /// Sum of all observations.
    #[must_use]
    pub fn sum(&self) -> f64 {
        self.observations
            .read()
            .unwrap_or_else(|poisoned| {
                warn!("histogram lock was poisoned, recovering");
                poisoned.into_inner()
            })
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
/// Prefer names with the `nebula_` prefix (e.g. `nebula_executions_total`)
/// for consistency and future Prometheus/OTLP export.
///
/// # Examples
///
/// ```
/// use nebula_telemetry::metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let counter = registry.counter("nebula_executions_total");
/// counter.inc();
/// assert_eq!(counter.get(), 1);
///
/// // Retrieving the same name returns the same metric.
/// let same = registry.counter("nebula_executions_total");
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
        let mut map = self.counters.write().unwrap_or_else(|poisoned| {
            warn!("counter registry lock was poisoned, recovering");
            poisoned.into_inner()
        });
        map.entry(name.to_owned()).or_default().clone()
    }

    /// Get or create a gauge by name.
    pub fn gauge(&self, name: &str) -> Gauge {
        let mut map = self.gauges.write().unwrap_or_else(|poisoned| {
            warn!("gauge registry lock was poisoned, recovering");
            poisoned.into_inner()
        });
        map.entry(name.to_owned()).or_default().clone()
    }

    /// Get or create a histogram by name.
    pub fn histogram(&self, name: &str) -> Histogram {
        let mut map = self.histograms.write().unwrap_or_else(|poisoned| {
            warn!("histogram registry lock was poisoned, recovering");
            poisoned.into_inner()
        });
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

    #[test]
    fn histogram_recovers_from_poisoned_lock() {
        let h = Histogram::new();
        h.observe(1.0);

        // Poison the RwLock by panicking while holding a write guard.
        let h2 = h.clone();
        let handle = std::thread::spawn(move || {
            let _guard = h2.observations.write().unwrap();
            panic!("intentional panic to poison histogram lock");
        });
        assert!(handle.join().is_err());

        // After poisoning, operations must recover.
        h.observe(2.0);
        assert!(h.count() >= 1);
        assert!(h.sum() >= 1.0);
    }

    #[test]
    fn registry_recovers_from_poisoned_lock() {
        let reg = MetricsRegistry::new();
        reg.counter("before");

        // Poison the counters lock.
        let inner = reg.counters.clone();
        let handle = std::thread::spawn(move || {
            let _guard = inner.write().unwrap();
            panic!("intentional panic to poison registry lock");
        });
        assert!(handle.join().is_err());

        // After poisoning, counter/gauge/histogram creation must recover.
        let c = reg.counter("after");
        c.inc();
        assert_eq!(c.get(), 1);

        let g = reg.gauge("g");
        g.set(42);
        assert_eq!(g.get(), 42);

        let h = reg.histogram("h");
        h.observe(1.0);
        assert_eq!(h.count(), 1);
    }
}
