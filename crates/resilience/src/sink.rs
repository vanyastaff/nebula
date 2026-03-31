//! `MetricsSink` — event sink for resilience observability.
//!
//! Replaces the custom `ObservabilityHook` system. The default is [`NoopSink`].
//! In nebula-engine, `EventBusSink` wraps nebula-eventbus — no direct dep here.

use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Duration;

/// A state in the circuit breaker state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CircuitState {
    /// Normal operation — requests pass through.
    Closed,
    /// Breaker tripped — requests rejected immediately.
    Open,
    /// Probing — limited requests allowed to test recovery.
    HalfOpen,
}

/// Events emitted by resilience patterns to the [`MetricsSink`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResilienceEvent {
    /// Circuit breaker transitioned between states.
    CircuitStateChanged {
        /// Previous circuit state.
        from: CircuitState,
        /// New circuit state.
        to: CircuitState,
    },
    /// A retry attempt was made.
    RetryAttempt {
        /// 1-based attempt number.
        attempt: u32,
        /// Whether another attempt will follow.
        will_retry: bool,
    },
    /// A bulkhead rejected a request (at capacity).
    BulkheadRejected,
    /// A timeout elapsed.
    TimeoutElapsed {
        /// Configured timeout duration.
        duration: Duration,
    },
    /// A hedge request was fired.
    HedgeFired {
        /// 1-based hedge request number.
        hedge_number: u32,
    },
    /// A rate limit was exceeded.
    RateLimitExceeded,
    /// Load shed — request rejected due to overload.
    LoadShed,
}

/// Fieldless discriminant of [`ResilienceEvent`] for type-safe event filtering.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResilienceEventKind {
    /// [`ResilienceEvent::CircuitStateChanged`]
    CircuitStateChanged,
    /// [`ResilienceEvent::RetryAttempt`]
    RetryAttempt,
    /// [`ResilienceEvent::BulkheadRejected`]
    BulkheadRejected,
    /// [`ResilienceEvent::TimeoutElapsed`]
    TimeoutElapsed,
    /// [`ResilienceEvent::HedgeFired`]
    HedgeFired,
    /// [`ResilienceEvent::RateLimitExceeded`]
    RateLimitExceeded,
    /// [`ResilienceEvent::LoadShed`]
    LoadShed,
}

/// Receives resilience events for observability (metrics, logging, `EventBus`).
///
/// This trait is designed to be implemented by downstream crates.
/// New methods will always have default implementations to avoid breaking changes.
pub trait MetricsSink: Send + Sync {
    /// Record a resilience event.
    fn record(&self, event: ResilienceEvent);
}

/// Default sink — discards all events. Zero cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSink;

impl MetricsSink for NoopSink {
    fn record(&self, _: ResilienceEvent) {}
}

/// Test sink — records all events for assertion.
#[derive(Debug, Default, Clone)]
pub struct RecordingSink {
    events: Arc<Mutex<Vec<ResilienceEvent>>>,
}

impl RecordingSink {
    /// Create a new empty recording sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot of all recorded events.
    #[must_use]
    pub fn events(&self) -> Vec<ResilienceEvent> {
        self.events.lock().clone()
    }

    /// Count events matching a given kind.
    #[must_use]
    pub fn count(&self, kind: ResilienceEventKind) -> usize {
        self.events
            .lock()
            .iter()
            .filter(|e| e.kind() == kind)
            .count()
    }

    /// Returns true if a `CircuitStateChanged` event to `to` was recorded.
    #[must_use]
    pub fn has_state_change(&self, to: CircuitState) -> bool {
        self.events()
            .iter()
            .any(|e| matches!(e, ResilienceEvent::CircuitStateChanged { to: t, .. } if *t == to))
    }
}

impl MetricsSink for RecordingSink {
    fn record(&self, event: ResilienceEvent) {
        self.events.lock().push(event);
    }
}

impl ResilienceEvent {
    /// Returns the fieldless discriminant of this event.
    #[must_use]
    pub const fn kind(&self) -> ResilienceEventKind {
        match self {
            Self::CircuitStateChanged { .. } => ResilienceEventKind::CircuitStateChanged,
            Self::RetryAttempt { .. } => ResilienceEventKind::RetryAttempt,
            Self::BulkheadRejected => ResilienceEventKind::BulkheadRejected,
            Self::TimeoutElapsed { .. } => ResilienceEventKind::TimeoutElapsed,
            Self::HedgeFired { .. } => ResilienceEventKind::HedgeFired,
            Self::RateLimitExceeded => ResilienceEventKind::RateLimitExceeded,
            Self::LoadShed => ResilienceEventKind::LoadShed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_sink_captures_events() {
        let sink = RecordingSink::new();
        sink.record(ResilienceEvent::BulkheadRejected);
        sink.record(ResilienceEvent::BulkheadRejected);
        assert_eq!(sink.count(ResilienceEventKind::BulkheadRejected), 2);
    }

    #[test]
    fn recording_sink_detects_state_change() {
        let sink = RecordingSink::new();
        sink.record(ResilienceEvent::CircuitStateChanged {
            from: CircuitState::Closed,
            to: CircuitState::Open,
        });
        assert!(sink.has_state_change(CircuitState::Open));
        assert!(!sink.has_state_change(CircuitState::HalfOpen));
    }

    #[test]
    fn noop_sink_does_not_panic() {
        let sink = NoopSink;
        sink.record(ResilienceEvent::LoadShed); // just must not panic
    }
}
