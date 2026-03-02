//! Observability counters for the event bus.

/// Counters exposed by [`EventBus::stats`](crate::EventBus::stats) for observability.
///
/// Metric names should use a consistent prefix (e.g. `nebula_*`) when exported
/// to Prometheus or OTLP.
#[derive(Debug, Clone, Default)]
pub struct EventBusStats {
    /// Total events successfully sent.
    pub sent_count: u64,
    /// Events dropped (no receivers, or back-pressure policy).
    pub dropped_count: u64,
    /// Current number of active subscribers.
    pub subscriber_count: usize,
}
