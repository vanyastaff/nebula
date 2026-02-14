//! Event bus for execution lifecycle events.
//!
//! Uses [`tokio::sync::broadcast`] for fan-out delivery to multiple subscribers.
//! Events are fire-and-forget projections -- dropping them is acceptable.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

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

/// Broadcast-based event bus.
///
/// Delivers events to all active subscribers. If no subscribers are
/// listening, events are silently dropped (fire-and-forget).
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
pub struct EventBus {
    sender: broadcast::Sender<ExecutionEvent>,
    emitted: AtomicU64,
}

impl EventBus {
    /// Create a new event bus with the given channel capacity.
    ///
    /// When the channel is full, the oldest events are dropped (lagging
    /// subscribers will see a `RecvError::Lagged`).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            emitted: AtomicU64::new(0),
        }
    }

    /// Emit an event to all subscribers.
    ///
    /// Returns silently if there are no active subscribers.
    pub fn emit(&self, event: ExecutionEvent) {
        self.emitted.fetch_add(1, Ordering::Relaxed);
        // Ignore send error (no active receivers).
        let _ = self.sender.send(event);
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> EventSubscriber {
        EventSubscriber {
            receiver: self.sender.subscribe(),
        }
    }

    /// Total number of events emitted since creation.
    #[must_use]
    pub fn total_emitted(&self) -> u64 {
        self.emitted.load(Ordering::Relaxed)
    }

    /// Number of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

/// Subscription handle for receiving events from the [`EventBus`].
pub struct EventSubscriber {
    receiver: broadcast::Receiver<ExecutionEvent>,
}

impl EventSubscriber {
    /// Receive the next event, waiting asynchronously.
    ///
    /// Returns `None` if the sender has been dropped or the subscriber
    /// has lagged (missed events due to buffer overflow).
    pub async fn recv(&mut self) -> Option<ExecutionEvent> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Try to receive an event without blocking.
    ///
    /// Returns `None` if no event is immediately available.
    pub fn try_recv(&mut self) -> Option<ExecutionEvent> {
        loop {
            match self.receiver.try_recv() {
                Ok(event) => return Some(event),
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        }
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
        assert_eq!(bus.total_emitted(), 1);
        assert_eq!(bus.subscriber_count(), 0);
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
