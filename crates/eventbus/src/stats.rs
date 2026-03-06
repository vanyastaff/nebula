//! Observability counters for the event bus.

/// Counters exposed by [`EventBus::stats`](crate::EventBus::stats) for observability.
///
/// Metric names should use a consistent prefix (e.g. `nebula_*`) when exported
/// to Prometheus or OTLP.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventBusStats {
    /// Total events successfully sent.
    pub sent_count: u64,
    /// Events dropped (no receivers, or back-pressure policy).
    pub dropped_count: u64,
    /// Current number of active subscribers.
    pub subscriber_count: usize,
}

impl EventBusStats {
    /// Total publish attempts observed by the bus.
    #[must_use]
    pub const fn total_attempts(&self) -> u64 {
        self.sent_count + self.dropped_count
    }

    /// Fraction of dropped events in range `0.0..=1.0`.
    #[must_use]
    pub fn drop_ratio(&self) -> f64 {
        let total = self.total_attempts();
        if total == 0 {
            return 0.0;
        }
        self.dropped_count as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_attempts_is_sum_of_sent_and_dropped() {
        let stats = EventBusStats {
            sent_count: 7,
            dropped_count: 3,
            subscriber_count: 2,
        };

        assert_eq!(stats.total_attempts(), 10);
    }

    #[test]
    fn drop_ratio_is_zero_without_attempts() {
        let stats = EventBusStats::default();
        assert_eq!(stats.drop_ratio(), 0.0);
    }

    #[test]
    fn drop_ratio_is_computed_from_totals() {
        let stats = EventBusStats {
            sent_count: 8,
            dropped_count: 2,
            subscriber_count: 1,
        };

        assert!((stats.drop_ratio() - 0.2).abs() < f64::EPSILON);
    }
}
