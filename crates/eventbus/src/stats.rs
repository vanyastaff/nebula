//! Observability counters for the event bus.

/// Counters exposed by [`EventBus::stats`](crate::EventBus::stats) for observability.
///
/// Metric names should use a consistent prefix (e.g. `nebula_*`) when exported
/// to Prometheus or OTLP.
///
/// # `dropped_count` semantics
///
/// `dropped_count` aggregates two independent signals:
///
/// 1. **Emit-time drops** (immediate): no subscribers, rejection by
///    [`BackPressurePolicy::DropNewest`](crate::BackPressurePolicy::DropNewest), or timeout under
///    [`BackPressurePolicy::Block`](crate::BackPressurePolicy::Block).
/// 2. **Recv-time lag** (eventually consistent): under
///    [`BackPressurePolicy::DropOldest`](crate::BackPressurePolicy::DropOldest) each subscriber
///    attributes the exact `RecvError::Lagged(n)` count tokio reports when it next pulls, so the
///    counter only updates as subscribers consume. With **N** subscribers each missing the same
///    evicted event, the counter increments by **N** (per-subscriber-event drop count, not unique
///    slot evictions).
///
/// Consequently `sent_count + dropped_count` may exceed the number of `emit()`
/// calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventBusStats {
    /// Total events accepted into the broadcast buffer.
    pub sent_count: u64,
    /// Events lost from a subscriber's perspective. See the type-level docs
    /// for the two contributing signals (emit-time drops + recv-time lag).
    pub dropped_count: u64,
    /// Current number of active subscribers.
    pub subscriber_count: usize,
}

impl EventBusStats {
    /// Sum of `sent_count` and `dropped_count`.
    ///
    /// Per the type-level docs, this exceeds the number of `emit()` calls when
    /// `DropOldest` overflow is observed by subscribers (each lag observation
    /// contributes to `dropped_count` while the emit itself contributed to
    /// `sent_count`).
    #[must_use]
    pub const fn total_attempts(&self) -> u64 {
        self.sent_count + self.dropped_count
    }

    /// Fraction of dropped events relative to [`total_attempts`](Self::total_attempts).
    ///
    /// Range is `0.0..=1.0`. Under sustained single-subscriber `DropOldest`
    /// saturation the ratio approaches `0.5`; with N lagging subscribers the
    /// ratio can approach `N/(N+1)`.
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
