//! Adapter over `nebula-telemetry::MetricsRegistry` with standard `nebula_*` names.
//!
//! Use this when you want to record metrics with the unified naming convention
//! without changing the underlying telemetry registry.

use std::sync::Arc;

use nebula_eventbus::EventBusStats;
use nebula_telemetry::{
    labels::LabelSet,
    metrics::{Counter, Gauge, Histogram, MetricsRegistry},
};

use crate::{filter::LabelAllowlist, naming::*};

/// Adapter that exposes telemetry metrics under standard `nebula_*` names.
///
/// Holds an `Arc<MetricsRegistry>` and provides accessors that return
/// counters/gauges/histograms with the canonical names. Engine and runtime
/// can use this to record metrics that are ready for Prometheus/OTLP export.
///
/// An optional [`LabelAllowlist`] can be configured via
/// [`TelemetryAdapter::with_allowlist`] to strip high-cardinality label keys
/// before they reach the registry.
#[derive(Clone, Debug)]
pub struct TelemetryAdapter {
    registry: Arc<MetricsRegistry>,
    allowlist: LabelAllowlist,
}

impl TelemetryAdapter {
    /// Create an adapter over the given telemetry registry.
    ///
    /// By default all labels are passed through. Use
    /// [`TelemetryAdapter::with_allowlist`] to restrict which keys are stored.
    #[must_use]
    pub fn new(registry: Arc<MetricsRegistry>) -> Self {
        Self {
            registry,
            allowlist: LabelAllowlist::all(),
        }
    }

    /// Attach a [`LabelAllowlist`] that filters label keys on every labeled
    /// accessor call.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use nebula_metrics::{LabelAllowlist, TelemetryAdapter};
    /// use nebula_telemetry::metrics::MetricsRegistry;
    ///
    /// let reg = Arc::new(MetricsRegistry::new());
    /// let adapter = TelemetryAdapter::new(Arc::clone(&reg)).with_allowlist(LabelAllowlist::only([
    ///     "action_type",
    ///     "status",
    ///     "trigger_type",
    /// ]));
    /// ```
    #[must_use]
    pub fn with_allowlist(mut self, allowlist: LabelAllowlist) -> Self {
        self.allowlist = allowlist;
        self
    }

    /// Apply the configured [`LabelAllowlist`] to `labels`.
    ///
    /// Use this to build a safe [`LabelSet`] before calling labeled accessors
    /// when the incoming label set may contain high-cardinality keys.
    #[must_use]
    pub fn filter_labels(&self, labels: &LabelSet) -> LabelSet {
        self.allowlist.apply(labels, self.registry.interner())
    }

    /// Underlying registry for custom or legacy metric names.
    #[must_use]
    pub fn registry(&self) -> &MetricsRegistry {
        &self.registry
    }

    // ---------- Workflow (engine) ----------

    /// Counter: workflow executions started.
    #[must_use]
    pub fn workflow_executions_started_total(&self) -> Counter {
        self.registry
            .counter(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL)
    }

    /// Counter: workflow executions completed successfully.
    #[must_use]
    pub fn workflow_executions_completed_total(&self) -> Counter {
        self.registry
            .counter(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL)
    }

    /// Counter: workflow executions failed.
    #[must_use]
    pub fn workflow_executions_failed_total(&self) -> Counter {
        self.registry
            .counter(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL)
    }

    /// Histogram: workflow execution duration in seconds.
    #[must_use]
    pub fn workflow_execution_duration_seconds(&self) -> Histogram {
        self.registry
            .histogram(NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS)
    }

    // ---------- Action (runtime) ----------

    /// Counter: action executions (success + failure).
    #[must_use]
    pub fn action_executions_total(&self) -> Counter {
        self.registry.counter(NEBULA_ACTION_EXECUTIONS_TOTAL)
    }

    /// Counter: action failures.
    #[must_use]
    pub fn action_failures_total(&self) -> Counter {
        self.registry.counter(NEBULA_ACTION_FAILURES_TOTAL)
    }

    /// Histogram: action execution duration in seconds.
    #[must_use]
    pub fn action_duration_seconds(&self) -> Histogram {
        self.registry.histogram(NEBULA_ACTION_DURATION_SECONDS)
    }

    // ---------- Generic access (for domain-specific names) ----------

    /// Get or create a counter by name. Prefer the typed accessors above when available.
    #[must_use]
    pub fn counter(&self, name: &str) -> Counter {
        self.registry.counter(name)
    }

    /// Get or create a gauge by name.
    #[must_use]
    pub fn gauge(&self, name: &str) -> Gauge {
        self.registry.gauge(name)
    }

    /// Get or create a histogram by name.
    #[must_use]
    pub fn histogram(&self, name: &str) -> Histogram {
        self.registry.histogram(name)
    }

    // ---------- Labeled variants of canonical action metrics ----------

    /// Counter: action executions with label dimensions (e.g. `action_type`).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use nebula_metrics::adapter::TelemetryAdapter;
    /// use nebula_telemetry::metrics::MetricsRegistry;
    ///
    /// let reg = Arc::new(MetricsRegistry::new());
    /// let adapter = TelemetryAdapter::new(Arc::clone(&reg));
    /// let labels = reg.interner().label_set(&[("action_type", "http.request")]);
    /// adapter.action_executions_labeled(&labels).inc();
    /// ```
    #[must_use]
    pub fn action_executions_labeled(&self, labels: &LabelSet) -> Counter {
        self.registry
            .counter_labeled(NEBULA_ACTION_EXECUTIONS_TOTAL, labels)
    }

    /// Counter: action failures with label dimensions.
    #[must_use]
    pub fn action_failures_labeled(&self, labels: &LabelSet) -> Counter {
        self.registry
            .counter_labeled(NEBULA_ACTION_FAILURES_TOTAL, labels)
    }

    /// Histogram: action execution duration with label dimensions.
    #[must_use]
    pub fn action_duration_labeled(&self, labels: &LabelSet) -> Histogram {
        self.registry
            .histogram_labeled(NEBULA_ACTION_DURATION_SECONDS, labels)
    }

    /// Counter: workflow executions started with label dimensions.
    #[must_use]
    pub fn workflow_executions_started_labeled(&self, labels: &LabelSet) -> Counter {
        self.registry
            .counter_labeled(NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL, labels)
    }

    /// Counter: workflow executions completed with label dimensions.
    #[must_use]
    pub fn workflow_executions_completed_labeled(&self, labels: &LabelSet) -> Counter {
        self.registry
            .counter_labeled(NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL, labels)
    }

    /// Counter: workflow executions failed with label dimensions.
    #[must_use]
    pub fn workflow_executions_failed_labeled(&self, labels: &LabelSet) -> Counter {
        self.registry
            .counter_labeled(NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, labels)
    }

    /// Access the underlying label interner to build [`LabelSet`]s.
    #[must_use]
    pub fn interner(&self) -> &nebula_telemetry::labels::LabelInterner {
        self.registry.interner()
    }

    // ---------- EventBus snapshots ----------

    /// Gauge: eventbus sent events snapshot.
    #[must_use]
    pub fn eventbus_sent(&self) -> Gauge {
        self.registry.gauge(NEBULA_EVENTBUS_SENT)
    }

    /// Gauge: eventbus dropped events snapshot.
    #[must_use]
    pub fn eventbus_dropped(&self) -> Gauge {
        self.registry.gauge(NEBULA_EVENTBUS_DROPPED)
    }

    /// Gauge: eventbus active subscriber snapshot.
    #[must_use]
    pub fn eventbus_subscribers(&self) -> Gauge {
        self.registry.gauge(NEBULA_EVENTBUS_SUBSCRIBERS)
    }

    /// Gauge: eventbus drop ratio snapshot in parts-per-million (`0..=1_000_000`).
    #[must_use]
    pub fn eventbus_drop_ratio_ppm(&self) -> Gauge {
        self.registry.gauge(NEBULA_EVENTBUS_DROP_RATIO_PPM)
    }

    /// Records an [`EventBusStats`] snapshot under standard `nebula_eventbus_*` gauges.
    pub fn record_eventbus_stats(&self, stats: &EventBusStats) {
        self.eventbus_sent().set(clamp_u64_to_i64(stats.sent_count));
        self.eventbus_dropped()
            .set(clamp_u64_to_i64(stats.dropped_count));
        self.eventbus_subscribers()
            .set(clamp_usize_to_i64(stats.subscriber_count));

        let ppm = (stats.drop_ratio() * 1_000_000.0).round();
        let ppm = if ppm.is_finite() {
            ppm.clamp(0.0, i64::MAX as f64) as i64
        } else {
            0
        };
        self.eventbus_drop_ratio_ppm().set(ppm);
    }
}

fn clamp_u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn clamp_usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_eventbus::EventBusStats;
    use nebula_telemetry::metrics::MetricsRegistry;

    use super::TelemetryAdapter;

    #[test]
    fn adapter_records_under_standard_names() {
        let registry = Arc::new(MetricsRegistry::new());
        let adapter = TelemetryAdapter::new(Arc::clone(&registry));

        adapter.workflow_executions_started_total().inc();
        adapter.workflow_executions_started_total().inc();
        adapter.action_executions_total().inc();
        adapter.action_failures_total().inc();

        assert_eq!(adapter.workflow_executions_started_total().get(), 2);
        assert_eq!(adapter.action_executions_total().get(), 1);
        assert_eq!(adapter.action_failures_total().get(), 1);
    }

    #[test]
    fn adapter_records_eventbus_snapshot_metrics() {
        let registry = Arc::new(MetricsRegistry::new());
        let adapter = TelemetryAdapter::new(Arc::clone(&registry));

        let stats = EventBusStats {
            sent_count: 75,
            dropped_count: 25,
            subscriber_count: 3,
        };

        adapter.record_eventbus_stats(&stats);

        assert_eq!(adapter.eventbus_sent().get(), 75);
        assert_eq!(adapter.eventbus_dropped().get(), 25);
        assert_eq!(adapter.eventbus_subscribers().get(), 3);
        assert_eq!(adapter.eventbus_drop_ratio_ppm().get(), 250_000);
    }

    #[test]
    fn record_eventbus_stats_handles_zero_totals_and_default_values() {
        let adapter = TelemetryAdapter::new(Arc::new(MetricsRegistry::new()));

        let stats = EventBusStats {
            sent_count: 0,
            dropped_count: 0,
            subscriber_count: 0,
        };
        tracing::debug!(
            "record_eventbus_stats test: sent={} dropped={} ppm_expected={}",
            stats.sent_count,
            stats.dropped_count,
            0
        );
        adapter.record_eventbus_stats(&stats);
        assert_eq!(adapter.eventbus_sent().get(), 0);
        assert_eq!(adapter.eventbus_dropped().get(), 0);
        assert_eq!(adapter.eventbus_subscribers().get(), 0);
        assert_eq!(adapter.eventbus_drop_ratio_ppm().get(), 0);

        let default_stats = EventBusStats::default();
        tracing::debug!(
            "record_eventbus_stats test: sent={} dropped={} ppm_expected={}",
            default_stats.sent_count,
            default_stats.dropped_count,
            0
        );
        adapter.record_eventbus_stats(&default_stats);
        assert_eq!(adapter.eventbus_sent().get(), 0);
        assert_eq!(adapter.eventbus_dropped().get(), 0);
        assert_eq!(adapter.eventbus_subscribers().get(), 0);
        assert_eq!(adapter.eventbus_drop_ratio_ppm().get(), 0);
    }

    #[test]
    fn record_eventbus_stats_handles_full_drop_ratio_and_rounding() {
        let adapter = TelemetryAdapter::new(Arc::new(MetricsRegistry::new()));

        let full_drop = EventBusStats {
            sent_count: 1_000_000,
            dropped_count: 1_000_000,
            subscriber_count: 42,
        };
        tracing::debug!(
            "record_eventbus_stats test: sent={} dropped={} ppm_expected={}",
            full_drop.sent_count,
            full_drop.dropped_count,
            1_000_000
        );
        adapter.record_eventbus_stats(&full_drop);
        assert_eq!(adapter.eventbus_drop_ratio_ppm().get(), 500_000);

        let fractional = EventBusStats {
            sent_count: 3,
            dropped_count: 1,
            subscriber_count: 1,
        };
        tracing::debug!(
            "record_eventbus_stats test: sent={} dropped={} ppm_expected={}",
            fractional.sent_count,
            fractional.dropped_count,
            250_000
        );
        adapter.record_eventbus_stats(&fractional);
        assert_eq!(adapter.eventbus_drop_ratio_ppm().get(), 250_000);
    }

    #[test]
    fn record_eventbus_stats_clamps_large_values_to_i64_max() {
        let adapter = TelemetryAdapter::new(Arc::new(MetricsRegistry::new()));

        let stats = EventBusStats {
            sent_count: u64::MAX,
            dropped_count: 0,
            subscriber_count: usize::MAX,
        };
        tracing::debug!(
            "record_eventbus_stats test: sent={} dropped={} ppm_expected={}",
            stats.sent_count,
            stats.dropped_count,
            0
        );
        adapter.record_eventbus_stats(&stats);

        assert_eq!(adapter.eventbus_sent().get(), i64::MAX);
        assert_eq!(adapter.eventbus_subscribers().get(), i64::MAX);
        assert_eq!(adapter.eventbus_drop_ratio_ppm().get(), 0);
    }
}
