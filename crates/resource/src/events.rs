//! Event broadcasting for resource lifecycle observability.
//!
//! Provides [`ResourceEvent`] variants and [`EventBus`] backed by `nebula-eventbus`.
//! Metrics and logging are controlled by app config (recorder/subscriber).

use std::time::Duration;

use crate::health::HealthState;
use crate::scope::Scope;

pub use nebula_eventbus::{BackPressurePolicy, EventBusStats, EventSubscriber};

/// Resource lifecycle event bus (wrapper around `nebula_eventbus::EventBus<ResourceEvent>`).
#[derive(Debug)]
pub struct EventBus(pub(crate) nebula_eventbus::EventBus<ResourceEvent>);

impl EventBus {
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        Self(nebula_eventbus::EventBus::new(buffer_size))
    }

    #[must_use]
    pub fn with_policy(buffer_size: usize, policy: BackPressurePolicy) -> Self {
        Self(nebula_eventbus::EventBus::with_policy(buffer_size, policy))
    }

    #[inline]
    pub fn emit(&self, event: ResourceEvent) {
        self.0.send(event);
    }

    pub async fn emit_async(&self, event: ResourceEvent) {
        self.0.send_async(event).await;
    }

    #[must_use]
    pub fn subscribe(&self) -> EventSubscriber<ResourceEvent> {
        self.0.subscribe()
    }

    #[must_use]
    pub fn stats(&self) -> EventBusStats {
        self.0.stats()
    }

    #[must_use]
    pub fn buffer_size(&self) -> usize {
        self.0.buffer_size()
    }

    #[must_use]
    pub fn policy(&self) -> &BackPressurePolicy {
        self.0.policy()
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.0.stats().subscriber_count
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self(nebula_eventbus::EventBus::default())
    }
}

// ---------------------------------------------------------------------------
// ResourceEvent (always present; no eventbus dependency)
// ---------------------------------------------------------------------------

/// Events emitted during resource lifecycle operations.
///
/// All variants carry a `resource_id` identifying the resource that
/// triggered the event. Subscribers receive cloned copies via [`EventBus::subscribe`].
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    /// A new resource was registered with the manager.
    Created {
        resource_id: String,
        scope: Scope,
    },
    /// A resource instance was successfully acquired from the pool.
    Acquired {
        resource_id: String,
        wait_duration: Duration,
    },
    /// A resource instance was released back to the pool.
    Released {
        resource_id: String,
        usage_duration: Duration,
    },
    /// The health state of a resource changed.
    HealthChanged {
        resource_id: String,
        from: HealthState,
        to: HealthState,
    },
    /// The pool is exhausted and a caller is waiting or was rejected.
    PoolExhausted {
        resource_id: String,
        waiters: usize,
    },
    /// A resource instance was cleaned up (permanently removed).
    CleanedUp {
        resource_id: String,
        reason: CleanupReason,
    },
    /// A resource was placed in quarantine.
    Quarantined {
        resource_id: String,
        reason: String,
    },
    /// A resource was released from quarantine.
    QuarantineReleased {
        resource_id: String,
        recovery_attempts: u32,
    },
    /// A resource's configuration was reloaded (hot-reload).
    ConfigReloaded {
        resource_id: String,
        scope: Scope,
    },
    /// An error occurred during a resource operation.
    Error {
        resource_id: String,
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
// Tests (eventbus-dependent tests only with "events" feature)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_creates_bus_with_1024_buffer() {
        let bus = EventBus::default();
        let _sub = bus.subscribe();
        assert_eq!(bus.buffer_size(), 1024);
    }

    #[test]
    fn emit_without_subscribers_does_not_panic() {
        let bus = EventBus::new(16);
        bus.emit(ResourceEvent::Created {
            resource_id: "test".to_string(),
            scope: Scope::Global,
        });
        let stats = bus.stats();
        assert_eq!(stats.dropped_count, 1);
    }

    #[tokio::test]
    async fn subscriber_receives_emitted_event() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe();

        bus.emit(ResourceEvent::Created {
            resource_id: "db".to_string(),
            scope: Scope::Global,
        });

        let event = sub.recv().await.expect("should receive event");
        match event {
            ResourceEvent::Created { resource_id, .. } => assert_eq!(resource_id, "db"),
            other => panic!("unexpected event: {other:?}"),
        }

        let stats = bus.stats();
        assert_eq!(stats.sent_count, 1);
        assert_eq!(stats.dropped_count, 0);
    }

    #[tokio::test]
    async fn config_reloaded_event_received() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe();

        bus.emit(ResourceEvent::ConfigReloaded {
            resource_id: "db".to_string(),
            scope: Scope::Global,
        });

        let event = sub.recv().await.expect("should receive event");
        assert!(matches!(event, ResourceEvent::ConfigReloaded { .. }));
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = EventBus::new(16);
        let mut sub1 = bus.subscribe();
        let mut sub2 = bus.subscribe();

        bus.emit(ResourceEvent::Error {
            resource_id: "redis".to_string(),
            error: "connection refused".to_string(),
        });

        let e1 = sub1.recv().await.unwrap();
        let e2 = sub2.recv().await.unwrap();
        assert!(matches!(e1, ResourceEvent::Error { .. }));
        assert!(matches!(e2, ResourceEvent::Error { .. }));
        assert_eq!(bus.stats().subscriber_count, 2);
    }

    #[test]
    fn drop_newest_policy_drops_without_subscribers() {
        let bus = EventBus::with_policy(4, BackPressurePolicy::DropNewest);
        bus.emit(ResourceEvent::Created {
            resource_id: "x".into(),
            scope: Scope::Global,
        });
        let stats = bus.stats();
        assert_eq!(stats.dropped_count, 1);
        assert_eq!(stats.sent_count, 0);
    }

    #[tokio::test]
    async fn drop_newest_policy_sends_with_subscriber() {
        let bus = EventBus::with_policy(4, BackPressurePolicy::DropNewest);
        let mut sub = bus.subscribe();

        bus.emit(ResourceEvent::Created {
            resource_id: "x".into(),
            scope: Scope::Global,
        });

        let event = sub.recv().await.expect("should receive event");
        assert!(matches!(event, ResourceEvent::Created { .. }));

        let stats = bus.stats();
        assert_eq!(stats.sent_count, 1);
        assert_eq!(stats.dropped_count, 0);
    }

    #[tokio::test]
    async fn block_policy_emit_async_succeeds_with_subscriber() {
        let bus = EventBus::with_policy(
            4,
            BackPressurePolicy::Block {
                timeout: Duration::from_millis(100),
            },
        );
        let mut sub = bus.subscribe();

        bus.emit_async(ResourceEvent::Created {
            resource_id: "y".into(),
            scope: Scope::Global,
        })
        .await;

        let event = sub.recv().await.expect("should receive event");
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

        bus.emit_async(ResourceEvent::Created {
            resource_id: "z".into(),
            scope: Scope::Global,
        })
        .await;

        let stats = bus.stats();
        assert_eq!(stats.dropped_count, 1);
    }

    #[test]
    fn event_bus_debug_output() {
        let bus = EventBus::with_policy(32, BackPressurePolicy::DropOldest);
        let debug = format!("{bus:?}");
        assert!(debug.contains("EventBus"));
        assert!(debug.contains("32"));
    }

    #[test]
    fn event_bus_stats_initial() {
        let bus = EventBus::new(8);
        let stats = bus.stats();
        assert_eq!(stats.sent_count, 0);
        assert_eq!(stats.dropped_count, 0);
        assert_eq!(stats.subscriber_count, 0);
    }

    #[test]
    fn back_pressure_policy_default_is_drop_oldest() {
        let policy = BackPressurePolicy::default();
        assert!(matches!(policy, BackPressurePolicy::DropOldest));
    }
}

#[cfg(test)]
mod tests_common {
    use super::*;

    #[test]
    fn cleanup_reason_is_clone() {
        let reason = CleanupReason::Expired;
        let cloned = reason.clone();
        assert!(matches!(cloned, CleanupReason::Expired));
    }

}
