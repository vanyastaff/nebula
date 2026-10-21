//! Concurrent registry for the primitive metric types.
//!
//! Names are interned via [`LabelInterner`] (backed by [`lasso::ThreadedRodeo`])
//! and stored against composite [`MetricKey`]s so labeled / unlabeled identities
//! share the same lookup path. The registry uses [`DashMap`] — a sharded
//! lock-free map — so recording metrics on hot paths does not block other
//! threads.
//!
//! Naming conventions and export formats live alongside this module
//! (`naming.rs`, `prometheus.rs`). The registry enforces the series-cardinality
//! policy from `filter.rs` ([`CardinalityLimits`]) whenever it creates a series.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::{DashMap, mapref::entry::Entry};
use lasso::Spur;

use crate::{
    counter::Counter,
    error::{MetricKind, MetricsError, MetricsResult},
    filter::{CardinalityLimits, OVERFLOW_LABEL_VALUE},
    gauge::Gauge,
    histogram::{DEFAULT_BUCKETS, Histogram},
    labels::{LabelInterner, LabelKey, LabelSet, MetricKey},
    naming::NEBULA_METRICS_SERIES_OVERFLOW_TOTAL,
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
/// # Cardinality
///
/// Every new series passes the registry's [`CardinalityLimits`] (default:
/// [`CardinalityLimits::default`]): denied label keys are stripped, and once a
/// metric holds `max_series_per_metric` label sets, a new label set resolves to
/// that metric's overflow series instead of a series of its own. Each folded
/// lookup increments [`NEBULA_METRICS_SERIES_OVERFLOW_TOTAL`], and the first
/// fold into an overflow series logs a warning. Existing series are unaffected.
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
    limits: Arc<CardinalityLimits>,
    /// `limits.denied_label_keys`, interned in `interner`.
    denied_label_keys: Arc<[LabelKey]>,
    /// Series per metric name that count toward the cap (labeled, not overflow).
    capped_series: Arc<DashMap<Spur, usize>>,
}

impl MetricsRegistry {
    /// Create a new empty registry with the default [`CardinalityLimits`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_cardinality_limits(CardinalityLimits::default())
    }

    /// Create a new empty registry that enforces `limits` on series creation.
    #[must_use]
    pub fn with_cardinality_limits(limits: CardinalityLimits) -> Self {
        let interner = LabelInterner::new();
        let denied_label_keys = Self::intern_denied_keys(&interner, &limits);
        Self {
            interner,
            series: Arc::new(DashMap::new()),
            limits: Arc::new(limits),
            denied_label_keys,
            capped_series: Arc::new(DashMap::new()),
        }
    }

    /// The cardinality policy this registry enforces.
    #[must_use]
    pub fn cardinality_limits(&self) -> &CardinalityLimits {
        &self.limits
    }

    fn intern_denied_keys(interner: &LabelInterner, limits: &CardinalityLimits) -> Arc<[LabelKey]> {
        limits
            .denied_label_keys()
            .iter()
            .map(|k| interner.intern(k))
            .collect()
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
        self.insert_counter(name_spur, &LabelSet::empty())
    }

    /// Get or create an unlabeled gauge by name.
    pub fn gauge(&self, name: &str) -> MetricsResult<Gauge> {
        let name_spur = self.interner.intern(name);
        self.insert_gauge(name_spur, &LabelSet::empty())
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
        self.insert_histogram(name_spur, &LabelSet::empty(), boundaries)
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
        self.insert_counter(name_spur, labels)
    }

    /// Get or create a gauge for the given metric name and label set.
    pub fn gauge_labeled(&self, name: &str, labels: &LabelSet) -> MetricsResult<Gauge> {
        let name_spur = self.interner.intern(name);
        self.insert_gauge(name_spur, labels)
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
        self.insert_histogram(name_spur, labels, &boundaries)
    }

    fn insert_counter(&self, name_spur: Spur, labels: &LabelSet) -> MetricsResult<Counter> {
        self.get_or_create(
            name_spur,
            labels,
            |series| match series {
                MetricSeries::Counter(c) => Ok(c.clone()),
                other => Err(self.kind_conflict(name_spur, MetricKind::Counter, other.kind())),
            },
            || {
                let c = Counter::new();
                (MetricSeries::Counter(c.clone()), c)
            },
        )
    }

    fn insert_gauge(&self, name_spur: Spur, labels: &LabelSet) -> MetricsResult<Gauge> {
        self.get_or_create(
            name_spur,
            labels,
            |series| match series {
                MetricSeries::Gauge(g) => Ok(g.clone()),
                other => Err(self.kind_conflict(name_spur, MetricKind::Gauge, other.kind())),
            },
            || {
                let g = Gauge::new();
                (MetricSeries::Gauge(g.clone()), g)
            },
        )
    }

    fn insert_histogram(
        &self,
        name_spur: Spur,
        labels: &LabelSet,
        boundaries: &[f64],
    ) -> MetricsResult<Histogram> {
        self.get_or_create(
            name_spur,
            labels,
            |series| match series {
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
            || {
                let h = Histogram::from_validated_boundaries(boundaries.to_vec());
                (MetricSeries::Histogram(h.clone()), h)
            },
        )
    }

    /// Look up the series for `(name, labels)`, creating it under the
    /// cardinality policy.
    ///
    /// Denied label keys are stripped first. A new labeled series takes one of
    /// its metric's `max_series_per_metric` slots; when none is left, the
    /// observation resolves to the metric's overflow series instead and the
    /// fold is counted.
    fn get_or_create<T>(
        &self,
        name: Spur,
        labels: &LabelSet,
        existing: impl Fn(&MetricSeries) -> MetricsResult<T>,
        create: impl Fn() -> (MetricSeries, T),
    ) -> MetricsResult<T> {
        let key = MetricKey::labeled(name, labels.without_keys(&self.denied_label_keys));
        let overflow_key = match self.series.entry(key) {
            Entry::Occupied(o) => return existing(o.get()),
            Entry::Vacant(v) => {
                if !Self::counts_toward_cap(&self.interner, &v.key().labels)
                    || self.take_series_slot(name)
                {
                    let (series, handle) = create();
                    v.insert(series);
                    return Ok(handle);
                }
                let overflow_value = self.interner.intern(OVERFLOW_LABEL_VALUE);
                MetricKey::labeled(name, v.key().labels.with_every_value(overflow_value))
            },
        };

        let handle = match self.series.entry(overflow_key) {
            Entry::Occupied(o) => existing(o.get())?,
            Entry::Vacant(v) => {
                tracing::warn!(
                    metric = self.interner.resolve(name),
                    max_series_per_metric = self.limits.max_series_per_metric(),
                    "metric reached its series cap; new label sets fold into the \
                     overflow series (label values \"{OVERFLOW_LABEL_VALUE}\")"
                );
                let (series, handle) = create();
                v.insert(series);
                handle
            },
        };
        self.record_series_overflow(name);
        Ok(handle)
    }

    /// Whether a series with `labels` takes one of its metric's capped slots.
    ///
    /// The unlabeled series and overflow series are always admitted.
    fn counts_toward_cap(interner: &LabelInterner, labels: &LabelSet) -> bool {
        if labels.is_empty() {
            return false;
        }
        interner
            .get(OVERFLOW_LABEL_VALUE)
            .is_none_or(|overflow| !labels.has_every_value(overflow))
    }

    /// Reserve one capped slot for metric `name`; `false` once the cap is reached.
    ///
    /// Called while the vacant series entry is held, so concurrent creators of
    /// the same series cannot both take a slot for it.
    fn take_series_slot(&self, name: Spur) -> bool {
        let mut count = self.capped_series.entry(name).or_insert(0);
        if *count >= self.limits.max_series_per_metric() {
            return false;
        }
        *count += 1;
        true
    }

    /// Count one fold into `name`'s overflow series.
    ///
    /// The counter is keyed by metric name, which is bounded by the code that
    /// names metrics, so it is created outside the cap.
    fn record_series_overflow(&self, name: Spur) {
        let key = MetricKey::labeled(
            self.interner.intern(NEBULA_METRICS_SERIES_OVERFLOW_TOTAL),
            self.interner.single("metric", self.interner.resolve(name)),
        );
        let counter = match self.series.entry(key) {
            Entry::Occupied(o) => match o.get() {
                MetricSeries::Counter(c) => c.clone(),
                _ => return,
            },
            Entry::Vacant(v) => {
                let c = Counter::new();
                v.insert(MetricSeries::Counter(c.clone()));
                c
            },
        };
        counter.inc();
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

        let capped_series: DashMap<Spur, usize> = DashMap::new();
        for entry in &new_series {
            if Self::counts_toward_cap(&new_interner, &entry.key().labels) {
                *capped_series.entry(entry.key().name).or_insert(0) += 1;
            }
        }

        self.denied_label_keys = Self::intern_denied_keys(&new_interner, &self.limits);
        self.interner = new_interner;
        self.series = Arc::new(new_series);
        self.capped_series = Arc::new(capped_series);
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
        // Compaction rebuilds the interner from live series (none left) plus
        // the denied label keys the cardinality policy keeps interned.
        assert_eq!(
            reg.interner_len(),
            reg.cardinality_limits().denied_label_keys().len()
        );
    }

    fn series_of(reg: &MetricsRegistry, name: &str) -> Vec<Vec<(String, String)>> {
        reg.snapshot_counters()
            .into_iter()
            .filter(|(key, _)| reg.interner().resolve(key.name) == name)
            .map(|(key, _)| {
                key.labels
                    .resolve(reg.interner())
                    .into_iter()
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .collect()
            })
            .collect()
    }

    fn overflow_count(reg: &MetricsRegistry, metric: &str) -> u64 {
        let labels = reg.interner().single("metric", metric);
        reg.counter_labeled(NEBULA_METRICS_SERIES_OVERFLOW_TOTAL, &labels)
            .unwrap()
            .get()
    }

    #[test]
    fn series_cap_folds_new_label_sets_into_overflow_series() {
        let reg = MetricsRegistry::with_cardinality_limits(
            CardinalityLimits::default().with_max_series_per_metric(2),
        );
        let tenant = |t: &str| reg.interner().label_set(&[("tenant", t), ("status", "ok")]);

        reg.counter_labeled("requests_total", &tenant("a"))
            .unwrap()
            .inc();
        reg.counter_labeled("requests_total", &tenant("b"))
            .unwrap()
            .inc();
        for i in 0..100 {
            reg.counter_labeled("requests_total", &tenant(&format!("flood-{i}")))
                .unwrap()
                .inc();
        }

        let series = series_of(&reg, "requests_total");
        assert_eq!(
            series.len(),
            3,
            "two admitted series plus one overflow: {series:?}"
        );
        let overflow = reg.interner().label_set(&[
            ("tenant", OVERFLOW_LABEL_VALUE),
            ("status", OVERFLOW_LABEL_VALUE),
        ]);
        assert_eq!(
            reg.counter_labeled("requests_total", &overflow)
                .unwrap()
                .get(),
            100,
            "every folded observation is still counted"
        );
        assert_eq!(overflow_count(&reg, "requests_total"), 100);

        // Admitted series keep resolving to themselves.
        let a = reg.counter_labeled("requests_total", &tenant("a")).unwrap();
        assert_eq!(a.get(), 1);
        assert_eq!(overflow_count(&reg, "requests_total"), 100);
    }

    #[test]
    fn series_cap_is_per_metric_and_skips_unlabeled_series() {
        let reg = MetricsRegistry::with_cardinality_limits(
            CardinalityLimits::default().with_max_series_per_metric(1),
        );
        reg.counter("a_total").unwrap().inc();
        let one = reg.interner().single("kind", "one");
        let two = reg.interner().single("kind", "two");
        reg.counter_labeled("a_total", &one).unwrap().inc();
        reg.counter_labeled("b_total", &one).unwrap().inc();
        reg.counter_labeled("b_total", &two).unwrap().inc();

        assert_eq!(
            series_of(&reg, "a_total").len(),
            2,
            "unlabeled + one labeled"
        );
        assert_eq!(overflow_count(&reg, "a_total"), 0);
        assert_eq!(overflow_count(&reg, "b_total"), 1);
    }

    #[test]
    fn series_cap_holds_under_concurrent_creation() {
        let reg = MetricsRegistry::with_cardinality_limits(
            CardinalityLimits::default().with_max_series_per_metric(10),
        );
        std::thread::scope(|scope| {
            for t in 0..8 {
                let reg = &reg;
                scope.spawn(move || {
                    for i in 0..50 {
                        let labels = reg.interner().single("k", &format!("{t}-{i}"));
                        reg.counter_labeled("racy_total", &labels).unwrap().inc();
                    }
                });
            }
        });
        assert_eq!(series_of(&reg, "racy_total").len(), 11);
        assert_eq!(overflow_count(&reg, "racy_total"), 400 - 10);
    }

    #[test]
    fn retain_recent_frees_series_slots() {
        let mut reg = MetricsRegistry::with_cardinality_limits(
            CardinalityLimits::default().with_max_series_per_metric(1),
        );
        let old = reg.interner().single("k", "old");
        reg.counter_labeled("m_total", &old).unwrap().inc();
        std::thread::sleep(Duration::from_millis(2));
        reg.retain_recent(Duration::ZERO);

        let new = reg.interner().single("k", "new");
        reg.counter_labeled("m_total", &new).unwrap().inc();
        assert_eq!(
            series_of(&reg, "m_total"),
            vec![vec![("k".to_owned(), "new".to_owned())]]
        );
    }

    #[test]
    fn default_limits_strip_identifier_label_keys() {
        let reg = MetricsRegistry::new();
        let with_id = reg.interner().label_set(&[
            ("status", "ok"),
            ("execution_id", "550e8400-e29b-41d4-a716-446655440000"),
            ("url", "https://example.com/hook?token=abc"),
        ]);
        let without_id = reg.interner().single("status", "ok");
        reg.counter_labeled("runs_total", &with_id).unwrap().inc();
        assert_eq!(
            reg.counter_labeled("runs_total", &without_id)
                .unwrap()
                .get(),
            1
        );
        assert_eq!(series_of(&reg, "runs_total").len(), 1);
    }

    #[test]
    fn empty_denied_keys_keep_every_label() {
        let reg = MetricsRegistry::with_cardinality_limits(
            CardinalityLimits::default().with_denied_label_keys(Vec::<String>::new()),
        );
        let labels = reg
            .interner()
            .label_set(&[("status", "ok"), ("execution_id", "e-1")]);
        reg.counter_labeled("runs_total", &labels).unwrap().inc();
        assert_eq!(series_of(&reg, "runs_total")[0].len(), 2);
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
