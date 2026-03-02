//! Event bus for execution lifecycle events.
//!
//! Backed by [`nebula_eventbus::EventBus`] for broadcast delivery with
//! configurable back-pressure. Events are fire-and-forget projections.

use std::sync::Arc;
use std::time::Duration;

use nebula_eventbus::EventBus as EventBusInner;
use serde::{Deserialize, Serialize};

/// Execution lifecycle event.
///
/// These events are emitted by the engine as executions progress.
/// They are projections, **not** the source of truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionEvent {
    /// An execution has started.
    Started {
        /// The execution identifier.
        execution_id: String,
        /// The workflow identifier.
        workflow_id: String,
    },
    /// A node within an execution has started.
    NodeStarted {
        /// The execution identifier.
        execution_id: String,
        /// The node identifier.
        node_id: String,
    },
    /// A node within an execution has completed.
    NodeCompleted {
        /// The execution identifier.
        execution_id: String,
        /// The node identifier.
        node_id: String,
        /// How long the node took.
        duration: Duration,
    },
    /// A node within an execution has failed.
    NodeFailed {
        /// The execution identifier.
        execution_id: String,
        /// The node identifier.
        node_id: String,
        /// Error description.
        error: String,
    },
    /// An execution has completed successfully.
    Completed {
        /// The execution identifier.
        execution_id: String,
        /// Total execution duration.
        duration: Duration,
    },
    /// An execution has failed.
    Failed {
        /// The execution identifier.
        execution_id: String,
        /// Error description.
        error: String,
    },
    /// An execution was cancelled.
    Cancelled {
        /// The execution identifier.
        execution_id: String,
    },
}

/// Broadcast-based event bus for execution events.
///
/// Wraps [`nebula_eventbus::EventBus<ExecutionEvent>`] so the telemetry API
/// stays stable (e.g. `total_emitted`, `subscriber_count`). Delivers events
/// to all active subscribers; when the buffer is full, behaviour is determined
/// by the eventbus back-pressure policy (default: drop oldest).
///
/// # Examples
///
/// ```
/// use nebula_telemetry::event::{EventBus, ExecutionEvent};
///
/// let bus = EventBus::new(64);
/// let mut sub = bus.subscribe();
///
/// bus.emit(ExecutionEvent::Started {
///     execution_id: "exec-1".into(),
///     workflow_id: "wf-1".into(),
/// });
///
/// // In async context: let event = sub.recv().await;
/// assert_eq!(bus.total_emitted(), 1);
/// ```
#[derive(Clone)]
pub struct EventBus(Arc<EventBusInner<ExecutionEvent>>);

/// Subscription handle for receiving events from the [`EventBus`].
pub type EventSubscriber = nebula_eventbus::Subscriber<ExecutionEvent>;

impl EventBus {
    /// Create a new event bus with the given channel capacity.
    ///
    /// Uses the eventbus default back-pressure policy (drop oldest when full).
    /// When the channel is full, lagging subscribers may see `Lagged` and skip events.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self(Arc::new(EventBusInner::new(capacity)))
    }

    /// Emit an event to all subscribers.
    ///
    /// Non-blocking; if there are no active subscribers, the event is dropped
    /// and counted in [`stats()`](Self::stats).
    #[inline]
    pub fn emit(&self, event: ExecutionEvent) {
        self.0.emit(event);
    }

    /// Subscribe to events.
    #[must_use]
    pub fn subscribe(&self) -> EventSubscriber {
        self.0.subscribe()
    }

    /// Total number of events successfully sent since creation.
    #[must_use]
    pub fn total_emitted(&self) -> u64 {
        self.0.stats().sent_count
    }

    /// Number of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.0.stats().subscriber_count
    }

    /// Snapshot of bus statistics (sent, dropped, subscriber count).
    #[must_use]
    pub fn stats(&self) -> nebula_eventbus::EventBusStats {
        self.0.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_without_subscribers_does_not_panic() {
        let bus = EventBus::new(16);
        bus.emit(ExecutionEvent::Started {
            execution_id: "e1".into(),
            workflow_id: "w1".into(),
        });
        // With no subscribers, eventbus counts as dropped, not sent
        assert_eq!(bus.subscriber_count(), 0);
        let stats = bus.stats();
        assert!(stats.sent_count == 0 && stats.dropped_count == 1);
    }

    #[test]
    fn subscriber_receives_via_try_recv() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe();

        bus.emit(ExecutionEvent::Cancelled {
            execution_id: "e1".into(),
        });

        let event = sub.try_recv().expect("should receive event");
        assert_eq!(
            event,
            ExecutionEvent::Cancelled {
                execution_id: "e1".into()
            }
        );
        assert_eq!(bus.total_emitted(), 1);
    }

    #[tokio::test]
    async fn subscriber_receives_via_recv() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe();

        bus.emit(ExecutionEvent::Completed {
            execution_id: "e1".into(),
            duration: Duration::from_secs(5),
        });

        let event = sub.recv().await.expect("should receive event");
        match event {
            ExecutionEvent::Completed {
                execution_id,
                duration,
            } => {
                assert_eq!(execution_id, "e1");
                assert_eq!(duration, Duration::from_secs(5));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn multiple_subscribers_each_get_a_copy() {
        let bus = EventBus::new(16);
        let mut sub1 = bus.subscribe();
        let mut sub2 = bus.subscribe();

        bus.emit(ExecutionEvent::Started {
            execution_id: "e1".into(),
            workflow_id: "w1".into(),
        });

        assert!(sub1.try_recv().is_some());
        assert!(sub2.try_recv().is_some());
    }

    #[test]
    fn subscriber_count_tracks_active_subscriptions() {
        let bus = EventBus::new(16);
        assert_eq!(bus.subscriber_count(), 0);

        let _sub1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);

        let _sub2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);

        drop(_sub1);
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[test]
    fn execution_event_serialization_roundtrip() {
        let events = vec![
            ExecutionEvent::Started {
                execution_id: "e1".into(),
                workflow_id: "w1".into(),
            },
            ExecutionEvent::Completed {
                execution_id: "e1".into(),
                duration: Duration::from_millis(1500),
            },
            ExecutionEvent::Failed {
                execution_id: "e1".into(),
                error: "timeout".into(),
            },
            ExecutionEvent::Cancelled {
                execution_id: "e1".into(),
            },
        ];

        for event in events {
            let json = serde_json::to_string(&event).expect("serialize");
            let roundtrip: ExecutionEvent = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(event, roundtrip);
        }
    }
}
