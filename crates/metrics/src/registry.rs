//! Concurrent registry for the primitive metric types.
//!
//! Names are interned via [`LabelInterner`] (backed by [`lasso::ThreadedRodeo`])
//! and stored against composite [`MetricKey`]s so labeled / unlabeled identities
//! share the same lookup path. The registry uses [`DashMap`] — a sharded
//! lock-free map — so recording metrics on hot paths does not block other
//! threads.
//!
//! Naming conventions and export formats live alongside this module
//! (`naming.rs`, `prometheus.rs`); the registry itself is pure storage.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::{DashMap, mapref::entry::Entry};
use lasso::Spur;

use crate::{
    counter::Counter,
    error::{MetricKind, MetricsError, MetricsResult},
    gauge::Gauge,
    histogram::{DEFAULT_BUCKETS, Histogram},
    labels::{LabelInterner, LabelSet, MetricKey},
};

/// Returns the current time as milliseconds since the Unix epoch.
///
/// Wall-clock steps backward ([`SystemTime::duration_since`] failure) map to
/// zero duration, so timestamps can move backward and interact poorly with
/// [`MetricsRegistry::retain_recent`]. Callers should not rely on strict
/// monotonicity of this clock. Crate-internal so the per-primitive modules can
/// stamp their `last_updated_ms` fields without each defining their own clock.
#[inline]
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// Stored series for one `(metric name, label set)` identity.
#[derive(Debug, Clone)]
enum MetricSeries {
    Counter(Counter),
    Gauge(Gauge),
    Histogram(Histogram),
}

impl MetricSeries {
    fn kind(&self) -> MetricKind {
        match self {
            Self::Counter(_) => MetricKind::Counter,
            Self::Gauge(_) => MetricKind::Gauge,
            Self::Histogram(_) => MetricKind::Histogram,
        }
    }

    fn last_updated_ms(&self) -> u64 {
        match self {
            Self::Counter(c) => c.last_updated_ms(),
            Self::Gauge(g) => g.last_updated_ms(),
            Self::Histogram(h) => h.last_updated_ms(),
        }
    }
}

/// Registry for creating and retrieving named metrics.
///
/// Metric names are interned via [`LabelInterner`] (backed by
/// [`lasso::ThreadedRodeo`]) so repeated lookups for the same name pay only
/// an integer comparison after the first call, not a string allocation.
///
/// Concurrent access uses [`DashMap`] — a sharded lock-free map — so
/// recording metrics on hot paths does not block other threads.
///
/// # Snapshots
///
/// [`Self::snapshot_counters`] and [`Self::snapshot_gauges`] return **live handles**
/// and enumerate registry membership approximately; values may change after the
/// call returns (weak / best-effort view).
///
/// For histograms exposed from [`Self::snapshot_histograms`], use
/// [`Histogram::snapshot`] before export so count, sum, and bucket totals reflect
/// one logical scrape instant (seqlock-backed); the returned [`Histogram`] handle
/// remains live afterward.
///
/// # Examples
///
/// ```
/// use nebula_metrics::MetricsRegistry;
///
/// let registry = MetricsRegistry::new();
/// let counter = registry.counter("request_total").unwrap();
/// counter.inc();
/// assert_eq!(counter.get(), 1);
///
/// let same = registry.counter("request_total").unwrap();
/// assert_eq!(same.get(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    /// Shared string interner — both metric names and label keys/values.
    pub(crate) interner: LabelInterner,
    series: Arc<DashMap<MetricKey, MetricSeries>>,
}

impl MetricsRegistry {
    /// Create a new empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            interner: LabelInterner::new(),
            series: Arc::new(DashMap::new()),
        }
    }

    fn resolve_metric_name(&self, name: Spur) -> String {
        self.interner.resolve(name).to_owned()
    }

    fn kind_conflict(&self, name: Spur, expected: MetricKind, actual: MetricKind) -> MetricsError {
        MetricsError::MetricKindConflict {
            metric_name: self.resolve_metric_name(name),
            expected_kind: expected,
            actual_kind: actual,
        }
    }

    /// Access the underlying label interner.
    ///
    /// Use this to build [`LabelSet`]s that are compatible with the labeled
    /// metric accessors on this registry.
    #[must_use]
    pub fn interner(&self) -> &LabelInterner {
        &self.interner
    }

    // ── Unlabeled accessors ─────────────────────────────────────────────────

    /// Get or create an unlabeled counter by name.
    pub fn counter(&self, name: &str) -> MetricsResult<Counter> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::unlabeled(name_spur);
        self.insert_counter(key, name_spur)
    }

    /// Get or create an unlabeled gauge by name.
    pub fn gauge(&self, name: &str) -> MetricsResult<Gauge> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::unlabeled(name_spur);
        self.insert_gauge(key, name_spur)
    }

    /// Get or create an unlabeled histogram using the built-in default bucket layout.
    pub fn histogram(&self, name: &str) -> MetricsResult<Histogram> {
        self.histogram_with_buckets_unlabeled(name, DEFAULT_BUCKETS)
    }

    fn histogram_with_buckets_unlabeled(
        &self,
        name: &str,
        boundaries: &[f64],
    ) -> MetricsResult<Histogram> {
        Histogram::validate_bucket_boundaries(boundaries)?;
        let name_spur = self.interner.intern(name);
        let key = MetricKey::unlabeled(name_spur);
        self.insert_histogram(key, name_spur, boundaries)
    }

    // ── Labeled accessors ───────────────────────────────────────────────────

    /// Get or create a counter for the given metric name and label set.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_metrics::MetricsRegistry;
    ///
    /// let reg = MetricsRegistry::new();
    /// let labels = reg.interner().label_set(&[("action_type", "http.request")]);
    /// let counter = reg
    ///     .counter_labeled("action_executions_total", &labels)
    ///     .unwrap();
    /// counter.inc();
    /// assert_eq!(counter.get(), 1);
    /// ```
    pub fn counter_labeled(&self, name: &str, labels: &LabelSet) -> MetricsResult<Counter> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::labeled(name_spur, labels.clone());
        self.insert_counter(key, name_spur)
    }

    /// Get or create a gauge for the given metric name and label set.
    pub fn gauge_labeled(&self, name: &str, labels: &LabelSet) -> MetricsResult<Gauge> {
        let name_spur = self.interner.intern(name);
        let key = MetricKey::labeled(name_spur, labels.clone());
        self.insert_gauge(key, name_spur)
    }

    /// Get or create a histogram for the given metric name and label set using
    /// the built-in default bucket layout.
    pub fn histogram_labeled(&self, name: &str, labels: &LabelSet) -> MetricsResult<Histogram> {
        self.histogram_with_buckets_labeled(name, labels, DEFAULT_BUCKETS.to_vec())
    }

    /// Get or create a histogram with custom bucket boundaries and a label set.
    ///
    /// Bucket boundaries are pinned at **first registration**. A later call
    /// with the same `(name, labels)` but different `boundaries` returns
    /// [`MetricsError::HistogramLayoutConflict`].
    pub fn histogram_with_buckets_labeled(
        &self,
        name: &str,
        labels: &LabelSet,
        boundaries: Vec<f64>,
    ) -> MetricsResult<Histogram> {
        Histogram::validate_bucket_boundaries(&boundaries)?;
        let name_spur = self.interner.intern(name);
        let key = MetricKey::labeled(name_spur, labels.clone());
        self.insert_histogram(key, name_spur, &boundaries)
    }

    fn insert_counter(&self, key: MetricKey, name_spur: Spur) -> MetricsResult<Counter> {
        match self.series.entry(key) {
            Entry::Occupied(o) => match o.get() {
                MetricSeries::Counter(c) => Ok(c.clone()),
                other => Err(self.kind_conflict(name_spur, MetricKind::Counter, other.kind())),
            },
            Entry::Vacant(v) => {
                let c = Counter::new();
                v.insert(MetricSeries::Counter(c.clone()));
                Ok(c)
            },
        }
    }

    fn insert_gauge(&self, key: MetricKey, name_spur: Spur) -> MetricsResult<Gauge> {
        match self.series.entry(key) {
            Entry::Occupied(o) => match o.get() {
                MetricSeries::Gauge(g) => Ok(g.clone()),
                other => Err(self.kind_conflict(name_spur, MetricKind::Gauge, other.kind())),
            },
            Entry::Vacant(v) => {
                let g = Gauge::new();
                v.insert(MetricSeries::Gauge(g.clone()));
                Ok(g)
            },
        }
    }

    fn insert_histogram(
        &self,
        key: MetricKey,
        name_spur: Spur,
        boundaries: &[f64],
    ) -> MetricsResult<Histogram> {
        match self.series.entry(key) {
            Entry::Occupied(o) => match o.get() {
                MetricSeries::Histogram(h) => {
                    if h.boundaries() != boundaries {
                        return Err(MetricsError::HistogramLayoutConflict {
                            metric_name: self.resolve_metric_name(name_spur),
                        });
                    }
                    Ok(h.clone())
                },
                other => Err(self.kind_conflict(name_spur, MetricKind::Histogram, other.kind())),
            },
            Entry::Vacant(v) => {
                let h = Histogram::from_validated_boundaries(boundaries.to_vec());
                v.insert(MetricSeries::Histogram(h.clone()));
                Ok(h)
            },
        }
    }

    // ── Snapshot / export ───────────────────────────────────────────────────

    /// Iterate all counter entries as `(MetricKey, Counter)` pairs.
    ///
    /// Used by exporters (Prometheus, OTLP) to serialize the current state.
    pub fn snapshot_counters(&self) -> Vec<(MetricKey, Counter)> {
        self.series
            .iter()
            .filter_map(|entry| match entry.value() {
                MetricSeries::Counter(c) => Some((entry.key().clone(), c.clone())),
                _ => None,
            })
            .collect()
    }

    /// Iterate all gauge entries as `(MetricKey, Gauge)` pairs.
    pub fn snapshot_gauges(&self) -> Vec<(MetricKey, Gauge)> {
        self.series
            .iter()
            .filter_map(|entry| match entry.value() {
                MetricSeries::Gauge(g) => Some((entry.key().clone(), g.clone())),
                _ => None,
            })
            .collect()
    }

    /// Iterate all histogram entries as `(MetricKey, Histogram)` pairs.
    pub fn snapshot_histograms(&self) -> Vec<(MetricKey, Histogram)> {
        self.series
            .iter()
            .filter_map(|entry| match entry.value() {
                MetricSeries::Histogram(h) => Some((entry.key().clone(), h.clone())),
                _ => None,
            })
            .collect()
    }

    // ── Expiration ──────────────────────────────────────────────────────────

    /// Remove all metric series that have not been updated within `max_age`.
    ///
    /// Reclaims stale series so dynamic labels do not grow the registry without
    /// bound. Stable metrics are naturally retained because they are written
    /// continuously.
    ///
    /// # Memory behavior
    ///
    /// After pruning, this compacts the label interner from still-live keys.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use nebula_metrics::MetricsRegistry;
    ///
    /// let mut reg = MetricsRegistry::new();
    /// let c = reg.counter("actions_total").unwrap();
    /// c.inc();
    ///
    /// reg.retain_recent(Duration::from_secs(300));
    /// assert_eq!(reg.metric_count(), 1);
    /// ```
    pub fn retain_recent(&mut self, max_age: Duration) {
        let age_ms: u64 = max_age.as_millis().try_into().unwrap_or(u64::MAX);
        let cutoff_ms = now_ms().saturating_sub(age_ms);
        self.series
            .retain(|_, series| series.last_updated_ms() >= cutoff_ms);
        self.compact_interner();
    }

    /// Total number of tracked metric series.
    #[must_use]
    pub fn metric_count(&self) -> usize {
        self.series.len()
    }

    /// Number of distinct strings held by the label interner.
    #[must_use]
    pub fn interner_len(&self) -> usize {
        self.interner.len()
    }

    fn compact_interner(&mut self) {
        let old_interner = self.interner.clone();
        let new_interner = LabelInterner::new();

        let remap_key = |key: &MetricKey| {
            let name = old_interner.resolve(key.name).to_owned();
            let labels_owned: Vec<(String, String)> = key
                .labels
                .resolve(&old_interner)
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect();
            let label_refs: Vec<(&str, &str)> = labels_owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            MetricKey::labeled(
                new_interner.intern(&name),
                new_interner.label_set(&label_refs),
            )
        };

        let new_series: DashMap<MetricKey, MetricSeries> = DashMap::new();
        for entry in &*self.series {
            let new_key = remap_key(entry.key());
            let cloned = match entry.value() {
                MetricSeries::Counter(c) => MetricSeries::Counter(c.clone()),
                MetricSeries::Gauge(g) => MetricSeries::Gauge(g.clone()),
                MetricSeries::Histogram(h) => MetricSeries::Histogram(h.clone()),
            };
            new_series.insert(new_key, cloned);
        }

        self.interner = new_interner;
        self.series = Arc::new(new_series);
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MetricKind, MetricsError};

    #[test]
    fn registry_returns_same_metric_for_same_name() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("requests").unwrap();
        c1.inc();
        let c2 = reg.counter("requests").unwrap();
        assert_eq!(c2.get(), 1);
    }

    #[test]
    fn registry_rejects_metric_kind_conflict() {
        let reg = MetricsRegistry::new();
        reg.counter("dup").unwrap().inc();
        let err = reg.gauge("dup").unwrap_err();
        assert!(matches!(
            err,
            MetricsError::MetricKindConflict {
                expected_kind: MetricKind::Gauge,
                actual_kind: MetricKind::Counter,
                ..
            }
        ));
    }

    #[test]
    fn registry_different_names_are_independent() {
        let reg = MetricsRegistry::new();
        let c1 = reg.counter("a").unwrap();
        let c2 = reg.counter("b").unwrap();
        c1.inc();
        assert_eq!(c1.get(), 1);
        assert_eq!(c2.get(), 0);
    }

    #[test]
    fn registry_labeled_counter_independent_from_unlabeled() {
        let reg = MetricsRegistry::new();
        let labels = reg.interner().label_set(&[("env", "prod")]);
        let labeled = reg.counter_labeled("requests_total", &labels).unwrap();
        let unlabeled = reg.counter("requests_total").unwrap();

        labeled.inc_by(10);
        unlabeled.inc_by(3);

        assert_eq!(labeled.get(), 10);
        assert_eq!(unlabeled.get(), 3);
    }

    #[test]
    fn registry_different_label_sets_are_independent() {
        let reg = MetricsRegistry::new();
        let prod = reg.interner().label_set(&[("env", "prod")]);
        let staging = reg.interner().label_set(&[("env", "staging")]);

        let c_prod = reg.counter_labeled("executions_total", &prod).unwrap();
        let c_staging = reg.counter_labeled("executions_total", &staging).unwrap();
        c_prod.inc_by(5);
        c_staging.inc_by(2);

        assert_eq!(c_prod.get(), 5);
        assert_eq!(c_staging.get(), 2);
    }

    #[test]
    fn registry_same_label_set_different_order_returns_same_metric() {
        let reg = MetricsRegistry::new();
        let ls1 = reg
            .interner()
            .label_set(&[("status", "ok"), ("action", "http.request")]);
        let ls2 = reg
            .interner()
            .label_set(&[("action", "http.request"), ("status", "ok")]);

        let c1 = reg
            .counter_labeled("action_executions_total", &ls1)
            .unwrap();
        c1.inc_by(7);
        let c2 = reg
            .counter_labeled("action_executions_total", &ls2)
            .unwrap();

        assert_eq!(
            c2.get(),
            7,
            "same label set in different order must return same metric"
        );
    }

    #[test]
    fn snapshot_returns_all_registered_metrics() {
        let reg = MetricsRegistry::new();
        reg.counter("c1").unwrap().inc();
        reg.counter("c2").unwrap().inc_by(3);
        reg.gauge("g1").unwrap().set(99);
        reg.histogram("h1").unwrap().observe(1.0);

        assert_eq!(reg.snapshot_counters().len(), 2);
        assert_eq!(reg.snapshot_gauges().len(), 1);
        assert_eq!(reg.snapshot_histograms().len(), 1);
    }

    #[test]
    fn metric_count_sums_all_types() {
        let reg = MetricsRegistry::new();
        reg.counter("c1").unwrap().inc();
        reg.gauge("g1").unwrap().set(1);
        reg.histogram("h1").unwrap().observe(1.0);
        assert_eq!(reg.metric_count(), 3);
    }

    #[test]
    fn histogram_with_buckets_labeled_errors_on_layout_conflict() {
        let reg = MetricsRegistry::new();
        let labels = reg.interner().label_set(&[("route", "/health")]);

        let first = reg
            .histogram_with_buckets_labeled("req_latency", &labels, vec![0.1, 0.5, 1.0])
            .unwrap();
        assert_eq!(first.boundaries(), &[0.1, 0.5, 1.0]);

        let err = reg
            .histogram_with_buckets_labeled("req_latency", &labels, vec![2.0, 4.0, 8.0])
            .unwrap_err();
        assert!(matches!(err, MetricsError::HistogramLayoutConflict { .. }));

        first.observe(0.3);
        let second = reg
            .histogram_with_buckets_labeled("req_latency", &labels, vec![0.1, 0.5, 1.0])
            .unwrap();
        assert_eq!(second.count(), 1);
    }

    #[test]
    fn interner_len_compacts_after_retain_recent() {
        let mut reg = MetricsRegistry::new();
        let labels = reg.interner().label_set(&[("k", "v1")]);
        reg.counter_labeled("m", &labels).unwrap().inc();
        let before = reg.interner_len();
        assert!(before >= 3); // at least "m", "k", "v1"

        std::thread::sleep(Duration::from_millis(2));
        reg.retain_recent(Duration::ZERO);
        assert_eq!(reg.metric_count(), 0);
        // Compaction rebuilds the interner from live series (none left).
        assert_eq!(reg.interner_len(), 0);
    }

    #[test]
    fn retain_recent_keeps_recently_updated_metrics() {
        let mut reg = MetricsRegistry::new();
        reg.counter("fresh").unwrap().inc();
        reg.gauge("also_fresh").unwrap().set(1);

        // Everything was just written — a 1-hour window should keep all.
        reg.retain_recent(Duration::from_hours(1));
        assert_eq!(reg.metric_count(), 2);
    }

    #[test]
    fn retain_recent_removes_stale_metrics() {
        let mut reg = MetricsRegistry::new();

        // Register a counter and immediately call retain_recent with zero
        // max_age so the cutoff is `now_ms()`.  Any metric whose timestamp
        // is strictly less than the cutoff will be removed.  Since we just
        // created the metrics they will have timestamps >= cutoff, so this
        // verifies the boundary condition: nothing is removed at max_age = 0.
        reg.counter("a").unwrap().inc();
        reg.gauge("b").unwrap().set(1);
        reg.histogram("c").unwrap().observe(0.5);
        reg.retain_recent(Duration::from_hours(1));
        assert_eq!(
            reg.metric_count(),
            3,
            "all metrics are recent, none should be removed"
        );

        // Now retain with max_age = 0: cutoff = now_ms(), so only metrics
        // updated at exactly now_ms() or later survive.  In practice all
        // metrics were created a few microseconds ago which means
        // their timestamp < cutoff, and they will be evicted.
        // We sleep 1 ms to ensure the cutoff is strictly after creation time.
        std::thread::sleep(Duration::from_millis(2));
        reg.retain_recent(Duration::ZERO);
        assert_eq!(
            reg.metric_count(),
            0,
            "all metrics should be evicted when max_age = 0"
        );
    }
}
