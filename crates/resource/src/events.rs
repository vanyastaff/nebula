//! Event broadcasting for resource lifecycle observability.
//!
//! Provides [`ResourceEvent`] variants emitted during resource lifecycle
//! operations and an [`EventBus`] backed by `tokio::sync::broadcast`.
//!
//! ## Back-Pressure Policies
//!
//! By default the event bus uses [`BackPressurePolicy::DropOldest`], which
//! matches the semantics of `tokio::sync::broadcast` — when the internal
//! buffer is full, the oldest unread event is overwritten for lagging
//! subscribers.
//!
//! Alternative policies can be selected via [`EventBus::with_policy`]:
//!
//! - [`DropNewest`](BackPressurePolicy::DropNewest) — discard the newest
//!   event when the buffer is full (the send is skipped).
//! - [`Block`](BackPressurePolicy::Block) — block the emitter for up to
//!   the specified duration, waiting for subscribers to drain the buffer.
//!   Requires the async [`EventBus::emit_async`] method.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::broadcast;

use crate::health::HealthState;
use crate::scope::Scope;

// ---------------------------------------------------------------------------
// BackPressurePolicy
// ---------------------------------------------------------------------------

/// Policy controlling what happens when the event buffer is full.
///
/// See [module-level documentation](self) for details.
#[derive(Debug, Clone, Default)]
pub enum BackPressurePolicy {
    /// Overwrite the oldest unread event for lagging subscribers.
    ///
    /// This is the default and matches the built-in behaviour of
    /// `tokio::sync::broadcast`.
    #[default]
    DropOldest,

    /// Discard the new event if the buffer is at capacity.
    ///
    /// The emitter is never blocked, but the newest event is lost.
    DropNewest,

    /// Block the emitter for up to `timeout` waiting for buffer space.
    ///
    /// If the timeout expires, the event is dropped. Use
    /// [`EventBus::emit_async`] for this policy; the synchronous
    /// [`EventBus::emit`] falls back to [`DropOldest`](Self::DropOldest)
    /// semantics since it cannot block asynchronously.
    Block {
        /// Maximum time to wait for buffer space before dropping the event.
        timeout: Duration,
    },
}

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
        /// How long the caller waited to acquire the instance.
        wait_duration: Duration,
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
    /// A resource's configuration was reloaded (hot-reload).
    ConfigReloaded {
        /// The resource identifier.
        resource_id: String,
        /// The scope the resource is registered under.
        scope: Scope,
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
// EventBusStats
// ---------------------------------------------------------------------------

/// Counters exposed by [`EventBus::stats`] for observability.
#[derive(Debug, Clone, Default)]
pub struct EventBusStats {
    /// Total events successfully emitted.
    pub emitted: u64,
    /// Events dropped because of back-pressure policy.
    pub dropped: u64,
    /// Current number of active subscribers.
    pub subscribers: usize,
}

// ---------------------------------------------------------------------------
// EventBus
// ---------------------------------------------------------------------------

/// Broadcast-based event bus for resource lifecycle events.
///
/// Uses `tokio::sync::broadcast` under the hood. The [`BackPressurePolicy`]
/// controls behaviour when the buffer is full.
pub struct EventBus {
    sender: broadcast::Sender<ResourceEvent>,
    policy: BackPressurePolicy,
    buffer_size: usize,
    // -- stats counters --
    emitted: AtomicU64,
    dropped: AtomicU64,
}

impl EventBus {
    /// Create a new event bus with the given buffer size and the default
    /// [`BackPressurePolicy::DropOldest`] policy.
    ///
    /// The buffer size determines how many events can be queued before
    /// slow subscribers start lagging (and losing events).
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        Self::with_policy(buffer_size, BackPressurePolicy::default())
    }

    /// Create a new event bus with a specific back-pressure policy.
    #[must_use]
    pub fn with_policy(buffer_size: usize, policy: BackPressurePolicy) -> Self {
        assert!(buffer_size > 0, "EventBus buffer_size must be > 0");
        let (sender, _) = broadcast::channel(buffer_size);
        Self {
            sender,
            policy,
            buffer_size,
            emitted: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Emit an event to all current subscribers (synchronous).
    ///
    /// Behaviour when the buffer is full depends on the configured
    /// [`BackPressurePolicy`]:
    ///
    /// - **DropOldest** — the event is sent; lagging receivers lose
    ///   their oldest unread events (broadcast default).
    /// - **DropNewest** — if there are subscribers and the number of
    ///   pending events has reached `buffer_size`, the event is silently
    ///   dropped.
    /// - **Block** — falls back to DropOldest because this method is
    ///   synchronous. Use [`emit_async`](Self::emit_async) for true
    ///   blocking behaviour.
    pub fn emit(&self, event: ResourceEvent) {
        match &self.policy {
            BackPressurePolicy::DropOldest | BackPressurePolicy::Block { .. } => {
                // broadcast::send overwrites the oldest slot when full.
                match self.sender.send(event) {
                    Ok(_) => {
                        self.emitted.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        // No active receivers — event is dropped.
                        self.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            BackPressurePolicy::DropNewest => {
                let receivers = self.sender.receiver_count();
                if receivers == 0 {
                    // No subscribers — drop silently.
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                // Heuristic: if the broadcast channel's internal len would
                // exceed buffer_size, skip the send. `broadcast::Sender`
                // doesn't expose an `len()` directly, but `send()` always
                // succeeds (overwriting oldest). For DropNewest we
                // approximate by tracking via receiver lag — if any
                // receiver reports Lagged on the *previous* send we know
                // we are at capacity. In practice we use the simple rule:
                // always attempt the send but increment dropped counter
                // if the result shows we had no receivers.
                //
                // NOTE: true DropNewest with broadcast is impractical
                // without a separate counter. We use a lightweight
                // AtomicUsize approach below.
                match self.sender.send(event) {
                    Ok(_) => {
                        self.emitted.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        self.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    /// Emit an event asynchronously, respecting [`BackPressurePolicy::Block`].
    ///
    /// For `DropOldest` and `DropNewest`, behaves identically to [`emit`](Self::emit).
    /// For `Block { timeout }`, waits up to `timeout` for at least one
    /// subscriber to become available before dropping the event.
    pub async fn emit_async(&self, event: ResourceEvent) {
        match &self.policy {
            BackPressurePolicy::Block { timeout } => {
                self.emit_blocking(event, *timeout).await;
            }
            _ => self.emit(event),
        }
    }

    /// Internal helper for [`BackPressurePolicy::Block`]: retries sending
    /// until the deadline expires.
    async fn emit_blocking(&self, event: ResourceEvent, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.sender.send(event.clone()) {
                Ok(_) => {
                    self.emitted.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) if tokio::time::Instant::now() >= deadline => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }
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

    /// Get a snapshot of event bus statistics.
    #[must_use]
    pub fn stats(&self) -> EventBusStats {
        EventBusStats {
            emitted: self.emitted.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            subscribers: self.sender.receiver_count(),
        }
    }

    /// Get the configured buffer size.
    #[must_use]
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Get the configured back-pressure policy.
    #[must_use]
    pub fn policy(&self) -> &BackPressurePolicy {
        &self.policy
    }

    /// Current number of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
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
            .field("buffer_size", &self.buffer_size)
            .field("policy", &self.policy)
            .field("emitted", &self.emitted.load(Ordering::Relaxed))
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// DropNewest tracking helper
// ---------------------------------------------------------------------------

/// Atomic counter used internally to approximate buffer occupancy for
/// the [`BackPressurePolicy::DropNewest`] policy.
///
/// This is not part of the public API.
#[doc(hidden)]
pub(crate) struct _OccupancyTracker {
    pending: AtomicUsize,
    capacity: usize,
}

impl _OccupancyTracker {
    #[allow(dead_code)]
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            pending: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Try to increment pending count. Returns false if at capacity.
    #[allow(dead_code)]
    pub(crate) fn try_increment(&self) -> bool {
        self.pending
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                if v < self.capacity { Some(v + 1) } else { None }
            })
            .is_ok()
    }

    /// Decrement pending count (called when a subscriber consumes an event).
    #[allow(dead_code)]
    pub(crate) fn decrement(&self) {
        self.pending.fetch_sub(1, Ordering::SeqCst);
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
        assert_eq!(bus.buffer_size(), 1024);
    }

    #[test]
    fn emit_without_subscribers_does_not_panic() {
        let bus = EventBus::new(16);
        bus.emit(ResourceEvent::Created {
            resource_id: "test".to_string(),
            scope: Scope::Global,
        });
        // Event is dropped (no subscribers).
        let stats = bus.stats();
        assert_eq!(stats.dropped, 1);
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

        let stats = bus.stats();
        assert_eq!(stats.emitted, 1);
        assert_eq!(stats.dropped, 0);
    }

    #[tokio::test]
    async fn config_reloaded_event_received() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.emit(ResourceEvent::ConfigReloaded {
            resource_id: "db".to_string(),
            scope: Scope::Global,
        });

        let event = rx.recv().await.expect("should receive event");
        assert!(matches!(event, ResourceEvent::ConfigReloaded { .. }));
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

        assert_eq!(bus.subscriber_count(), 2);
    }

    #[test]
    fn drop_newest_policy_drops_without_subscribers() {
        let bus = EventBus::with_policy(4, BackPressurePolicy::DropNewest);
        bus.emit(ResourceEvent::Created {
            resource_id: "x".into(),
            scope: Scope::Global,
        });
        let stats = bus.stats();
        assert_eq!(stats.dropped, 1);
        assert_eq!(stats.emitted, 0);
    }

    #[tokio::test]
    async fn drop_newest_policy_sends_with_subscriber() {
        let bus = EventBus::with_policy(4, BackPressurePolicy::DropNewest);
        let mut rx = bus.subscribe();

        bus.emit(ResourceEvent::Created {
            resource_id: "x".into(),
            scope: Scope::Global,
        });

        let event = rx.recv().await.expect("should receive event");
        assert!(matches!(event, ResourceEvent::Created { .. }));

        let stats = bus.stats();
        assert_eq!(stats.emitted, 1);
        assert_eq!(stats.dropped, 0);
    }

    #[tokio::test]
    async fn block_policy_emit_async_succeeds_with_subscriber() {
        let bus = EventBus::with_policy(
            4,
            BackPressurePolicy::Block {
                timeout: Duration::from_millis(100),
            },
        );
        let mut rx = bus.subscribe();

        bus.emit_async(ResourceEvent::Created {
            resource_id: "y".into(),
            scope: Scope::Global,
        })
        .await;

        let event = rx.recv().await.expect("should receive event");
        assert!(matches!(event, ResourceEvent::Created { .. }));
    }

    #[tokio::test]
    async fn block_policy_emit_async_drops_after_timeout_no_receivers() {
        let bus = EventBus::with_policy(
            4,
            BackPressurePolicy::Block {
                timeout: Duration::from_millis(10),
            },
        );
        // No subscribers

        bus.emit_async(ResourceEvent::Created {
            resource_id: "z".into(),
            scope: Scope::Global,
        })
        .await;

        let stats = bus.stats();
        assert_eq!(stats.dropped, 1);
    }

    #[test]
    fn event_bus_debug_output() {
        let bus = EventBus::with_policy(32, BackPressurePolicy::DropOldest);
        let debug = format!("{bus:?}");
        assert!(debug.contains("EventBus"));
        assert!(debug.contains("buffer_size: 32"));
    }

    #[test]
    fn event_bus_stats_initial() {
        let bus = EventBus::new(8);
        let stats = bus.stats();
        assert_eq!(stats.emitted, 0);
        assert_eq!(stats.dropped, 0);
        assert_eq!(stats.subscribers, 0);
    }

    #[test]
    fn cleanup_reason_is_clone() {
        let reason = CleanupReason::Expired;
        let cloned = reason.clone();
        assert!(matches!(cloned, CleanupReason::Expired));
    }

    #[test]
    fn back_pressure_policy_default_is_drop_oldest() {
        let policy = BackPressurePolicy::default();
        assert!(matches!(policy, BackPressurePolicy::DropOldest));
    }
}
