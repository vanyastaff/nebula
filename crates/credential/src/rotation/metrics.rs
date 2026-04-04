//! Registry-backed metrics for credential rotation.
//!
//! Replaces the previous `RwLock<MetricsInner>` implementation with lock-free
//! counters, gauges, and histograms backed by [`MetricsRegistry`]. Duration
//! percentiles are computed by the telemetry [`Histogram`] with O(1) bounded
//! buckets instead of the old O(n log n) sort-based approach.

use nebula_metrics::naming::{
    NEBULA_CREDENTIAL_ACTIVE_TOTAL, NEBULA_CREDENTIAL_EXPIRED_TOTAL,
    NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS, NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
    NEBULA_CREDENTIAL_ROTATIONS_TOTAL,
};
use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};

/// Registry-backed counters and histograms for credential rotation.
///
/// All methods are lock-free. Duration percentiles are computed by the
/// telemetry [`Histogram`] with O(1) bounded buckets.
///
/// # Examples
///
/// ```
/// use nebula_telemetry::metrics::MetricsRegistry;
/// use nebula_credential::rotation::RotationMetrics;
/// use std::time::Duration;
///
/// let registry = MetricsRegistry::new();
/// let metrics = RotationMetrics::new(&registry);
///
/// // Record a successful rotation that took 30 seconds.
/// metrics.record_rotation(Duration::from_secs(30), true);
///
/// assert_eq!(metrics.total_rotations(), 1);
/// assert_eq!(metrics.total_failures(), 0);
/// assert!((metrics.success_rate() - 1.0).abs() < f64::EPSILON);
/// ```
#[derive(Debug, Clone)]
pub struct RotationMetrics {
    rotations_total: Counter,
    failures_total: Counter,
    duration_seconds: Histogram,
    active_total: Gauge,
    expired_total: Counter,
}

impl RotationMetrics {
    /// Creates metrics backed by the given registry.
    #[must_use]
    pub fn new(registry: &MetricsRegistry) -> Self {
        Self {
            rotations_total: registry.counter(NEBULA_CREDENTIAL_ROTATIONS_TOTAL),
            failures_total: registry.counter(NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL),
            duration_seconds: registry.histogram(NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS),
            active_total: registry.gauge(NEBULA_CREDENTIAL_ACTIVE_TOTAL),
            expired_total: registry.counter(NEBULA_CREDENTIAL_EXPIRED_TOTAL),
        }
    }

    /// Records a rotation attempt with its duration and outcome.
    pub fn record_rotation(&self, duration: std::time::Duration, success: bool) {
        self.rotations_total.inc();
        self.duration_seconds.observe(duration.as_secs_f64());
        if !success {
            self.failures_total.inc();
        }
    }

    /// Records a rotation failure (without duration).
    pub fn record_failure(&self) {
        self.rotations_total.inc();
        self.failures_total.inc();
    }

    /// Sets the current number of active credentials.
    pub fn set_active(&self, count: i64) {
        self.active_total.set(count);
    }

    /// Records a credential expiration.
    pub fn record_expired(&self) {
        self.expired_total.inc();
    }

    /// Total rotation attempts.
    #[must_use]
    pub fn total_rotations(&self) -> u64 {
        self.rotations_total.get()
    }

    /// Total rotation failures.
    #[must_use]
    pub fn total_failures(&self) -> u64 {
        self.failures_total.get()
    }

    /// Success rate as `0.0..=1.0`. Returns `0.0` if no rotations recorded.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        let total = self.rotations_total.get();
        if total == 0 {
            return 0.0;
        }
        let failures = self.failures_total.get();
        (total.saturating_sub(failures)) as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn registry() -> MetricsRegistry {
        MetricsRegistry::new()
    }

    #[test]
    fn records_successful_rotation() {
        let reg = registry();
        let m = RotationMetrics::new(&reg);

        m.record_rotation(Duration::from_secs(30), true);

        assert_eq!(m.total_rotations(), 1);
        assert_eq!(m.total_failures(), 0);
        assert!((m.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn records_failed_rotation() {
        let reg = registry();
        let m = RotationMetrics::new(&reg);

        m.record_rotation(Duration::from_secs(5), false);

        assert_eq!(m.total_rotations(), 1);
        assert_eq!(m.total_failures(), 1);
        assert!((m.success_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn success_rate_mixed() {
        let reg = registry();
        let m = RotationMetrics::new(&reg);

        // 2 successes + 1 failure = 2/3 ≈ 0.666
        m.record_rotation(Duration::from_secs(10), true);
        m.record_rotation(Duration::from_secs(20), true);
        m.record_rotation(Duration::from_secs(5), false);

        assert_eq!(m.total_rotations(), 3);
        assert_eq!(m.total_failures(), 1);
        assert!((m.success_rate() - 2.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn success_rate_zero_rotations_returns_zero() {
        let reg = registry();
        let m = RotationMetrics::new(&reg);

        assert!((m.success_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn active_and_expired_tracking() {
        let reg = registry();
        let m = RotationMetrics::new(&reg);

        m.set_active(42);
        assert_eq!(m.active_total.get(), 42);

        m.record_expired();
        m.record_expired();
        assert_eq!(m.expired_total.get(), 2);

        // Active gauge can be updated independently.
        m.set_active(40);
        assert_eq!(m.active_total.get(), 40);
    }

    #[test]
    fn duration_recorded_in_histogram() {
        let reg = registry();
        let m = RotationMetrics::new(&reg);

        m.record_rotation(Duration::from_millis(500), true);
        m.record_rotation(Duration::from_secs(2), true);

        assert_eq!(m.duration_seconds.count(), 2);
        assert!((m.duration_seconds.sum() - 2.5).abs() < 1e-10);
    }

    #[test]
    fn record_failure_without_duration() {
        let reg = registry();
        let m = RotationMetrics::new(&reg);

        m.record_failure();

        assert_eq!(m.total_rotations(), 1);
        assert_eq!(m.total_failures(), 1);
        // No histogram observation expected.
        assert_eq!(m.duration_seconds.count(), 0);
    }
}
