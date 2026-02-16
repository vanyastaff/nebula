//! Event broadcasting for resource lifecycle observability.
//!
//! Provides [`ResourceEvent`] variants emitted during resource lifecycle
//! operations and an [`EventBus`] backed by `tokio::sync::broadcast`.

use std::time::Duration;

use tokio::sync::broadcast;

use crate::health::HealthState;
use crate::scope::Scope;

// ---------------------------------------------------------------------------
// ResourceEvent
// ---------------------------------------------------------------------------

/// Events emitted during resource lifecycle operations.
///
/// All variants carry a `resource_id` identifying the resource that
/// triggered the event. Subscribers receive cloned copies via
/// [`EventBus::subscribe`].
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    /// A new resource was registered with the manager.
    Created {
        /// The resource identifier.
        resource_id: String,
        /// The scope the resource was registered under.
        scope: Scope,
    },
    /// A resource instance was successfully acquired from the pool.
    Acquired {
        /// The resource identifier.
        resource_id: String,
    },
    /// A resource instance was released back to the pool.
    Released {
        /// The resource identifier.
        resource_id: String,
        /// How long the instance was held by the caller.
        usage_duration: Duration,
    },
    /// The health state of a resource changed.
    HealthChanged {
        /// The resource identifier.
        resource_id: String,
        /// Previous health state.
        from: HealthState,
        /// New health state.
        to: HealthState,
    },
    /// The pool is exhausted and a caller is waiting or was rejected.
    PoolExhausted {
        /// The resource identifier.
        resource_id: String,
        /// Number of callers currently waiting for an instance.
        waiters: usize,
    },
    /// A resource instance was cleaned up (permanently removed).
    CleanedUp {
        /// The resource identifier.
        resource_id: String,
        /// The reason the instance was cleaned up.
        reason: CleanupReason,
    },
    /// A resource was placed in quarantine.
    Quarantined {
        /// The resource identifier.
        resource_id: String,
        /// Human-readable reason for quarantine.
        reason: String,
    },
    /// A resource was released from quarantine.
    QuarantineReleased {
        /// The resource identifier.
        resource_id: String,
        /// How many recovery attempts it took.
        recovery_attempts: u32,
    },
    /// An error occurred during a resource operation.
    Error {
        /// The resource identifier.
        resource_id: String,
        /// Human-readable error description.
        error: String,
    },
}

// ---------------------------------------------------------------------------
// CleanupReason
// ---------------------------------------------------------------------------

/// Reason a resource instance was permanently removed from the pool.
#[derive(Debug, Clone)]
pub enum CleanupReason {
    /// The instance exceeded its maximum lifetime.
    Expired,
    /// The instance was idle longer than the configured timeout.
    IdleTimeout,
    /// A health check determined the instance is unhealthy.
    HealthCheckFailed,
    /// The pool is shutting down.
    Shutdown,
    /// The instance was evicted during maintenance.
    Evicted,
    /// Recycling the instance failed.
    RecycleFailed,
}

// ---------------------------------------------------------------------------
// EventBus
// ---------------------------------------------------------------------------

/// Broadcast-based event bus for resource lifecycle events.
///
/// Uses `tokio::sync::broadcast` under the hood. Emission is fire-and-forget:
/// if no subscribers are listening or the channel is full, events are silently
/// dropped (no backpressure on the emitter).
pub struct EventBus {
    sender: broadcast::Sender<ResourceEvent>,
}

impl EventBus {
    /// Create a new event bus with the given buffer size.
    ///
    /// The buffer size determines how many events can be queued before
    /// slow subscribers start lagging (and losing events).
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer_size);
        Self { sender }
    }

    /// Emit an event to all current subscribers.
    ///
    /// This is non-blocking. If there are no subscribers or the channel
    /// is full, the event is silently dropped.
    pub fn emit(&self, event: ResourceEvent) {
        // Ignore the error — it just means there are no active receivers.
        let _ = self.sender.send(event);
    }

    /// Subscribe to events.
    ///
    /// Returns a receiver that will get all events emitted after this
    /// call. If the subscriber falls behind by more than `buffer_size`
    /// events, it will receive a `Lagged` error and skip to the latest.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ResourceEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("subscriber_count", &self.sender.receiver_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_creates_bus_with_1024_buffer() {
        let bus = EventBus::default();
        // Can subscribe without panicking
        let _rx = bus.subscribe();
    }

    #[test]
    fn emit_without_subscribers_does_not_panic() {
        let bus = EventBus::new(16);
        bus.emit(ResourceEvent::Created {
            resource_id: "test".to_string(),
            scope: Scope::Global,
        });
    }

    #[tokio::test]
    async fn subscriber_receives_emitted_event() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit(ResourceEvent::Created {
            resource_id: "db".to_string(),
            scope: Scope::Global,
        });

        let event = rx.recv().await.expect("should receive event");
        match event {
            ResourceEvent::Created { resource_id, .. } => {
                assert_eq!(resource_id, "db");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.emit(ResourceEvent::Error {
            resource_id: "redis".to_string(),
            error: "connection refused".to_string(),
        });

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();

        assert!(matches!(e1, ResourceEvent::Error { .. }));
        assert!(matches!(e2, ResourceEvent::Error { .. }));
    }
}
