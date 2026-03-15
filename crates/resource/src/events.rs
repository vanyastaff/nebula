//! Event broadcasting for resource lifecycle observability.
//!
//! Provides [`ResourceEvent`] variants and [`EventBus`] backed by `nebula-eventbus`.
//! Metrics and logging are controlled by app config (recorder/subscriber).

use std::time::Duration;

use crate::health::HealthState;
use crate::scope::Scope;
use nebula_core::ResourceKey;

pub use nebula_eventbus::{
    BackPressurePolicy, EventBusStats, EventFilter, EventSubscriber, FilteredSubscriber,
    PublishOutcome, ScopedEvent, SubscriptionScope,
};

/// Resource lifecycle event bus (wrapper around `nebula_eventbus::EventBus<ResourceEvent>`).
#[derive(Debug, Default)]
pub struct EventBus(pub(crate) nebula_eventbus::EventBus<ResourceEvent>);

/// Scoped/filtering subscription handle for [`ResourceEvent`].
pub type ScopedSubscriber = FilteredSubscriber<ResourceEvent>;

impl EventBus {
    /// Creates a new event bus with the given buffer size (default back-pressure policy).
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        Self(nebula_eventbus::EventBus::new(buffer_size))
    }

    /// Creates a new event bus with the given buffer size and back-pressure policy.
    #[must_use]
    pub fn with_policy(buffer_size: usize, policy: BackPressurePolicy) -> Self {
        Self(nebula_eventbus::EventBus::with_policy(buffer_size, policy))
    }

    /// Sends an event synchronously (non-blocking; may drop if buffer full per policy).
    #[inline]
    pub fn emit(&self, event: ResourceEvent) -> PublishOutcome {
        self.0.emit(event)
    }

    /// Sends an event asynchronously (may block or timeout depending on policy).
    pub async fn emit_async(&self, event: ResourceEvent) -> PublishOutcome {
        self.0.emit_async(event).await
    }

    /// Returns a new subscriber that receives clones of emitted events.
    #[must_use]
    pub fn subscribe(&self) -> EventSubscriber<ResourceEvent> {
        self.0.subscribe()
    }

    /// Returns a filtered subscriber for custom predicates.
    #[must_use]
    pub fn subscribe_filtered(&self, filter: EventFilter<ResourceEvent>) -> ScopedSubscriber {
        self.0.subscribe_filtered(filter)
    }

    /// Returns a scoped subscriber based on workflow/execution/resource metadata.
    #[must_use]
    pub fn subscribe_scoped(&self, scope: SubscriptionScope) -> ScopedSubscriber {
        self.0.subscribe_scoped(scope)
    }

    /// Returns current bus statistics (sent, dropped, subscriber count).
    #[must_use]
    pub fn stats(&self) -> EventBusStats {
        self.0.stats()
    }

    /// Returns the configured buffer size.
    #[must_use]
    pub fn buffer_size(&self) -> usize {
        self.0.buffer_size()
    }

    /// Returns the back-pressure policy.
    #[must_use]
    pub fn policy(&self) -> &BackPressurePolicy {
        self.0.policy()
    }

    /// Returns the current number of active subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.0.stats().subscriber_count
    }
}

// ---------------------------------------------------------------------------
// ResourceEvent (always present; no eventbus dependency)
// ---------------------------------------------------------------------------

/// Events emitted during resource lifecycle operations.
///
/// All variants carry a [`ResourceKey`] identifying the resource type that
/// triggered the event. Subscribers receive cloned copies via [`EventBus::subscribe`].
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    /// A new resource was registered with the manager.
    Created {
        /// Resource key.
        resource_key: ResourceKey,
        /// Scope (e.g. global, workflow).
        scope: Scope,
    },
    /// A resource instance was successfully acquired from the pool.
    Acquired {
        /// Resource key.
        resource_key: ResourceKey,
        /// Time spent waiting for the instance.
        wait_duration: Duration,
    },
    /// A resource instance was released back to the pool.
    Released {
        /// Resource key.
        resource_key: ResourceKey,
        /// Time the instance was in use.
        usage_duration: Duration,
    },
    /// The health state of a resource changed.
    HealthChanged {
        /// Resource key.
        resource_key: ResourceKey,
        /// Previous health state.
        from: HealthState,
        /// New health state.
        to: HealthState,
    },
    /// The pool is exhausted and a caller is waiting or was rejected.
    PoolExhausted {
        /// Resource key.
        resource_key: ResourceKey,
        /// Number of waiters (or 0 if rejected).
        waiters: usize,
    },
    /// A resource instance was cleaned up (permanently removed).
    CleanedUp {
        /// Resource key.
        resource_key: ResourceKey,
        /// Reason for cleanup.
        reason: CleanupReason,
    },
    /// Circuit breaker opened for a resource operation.
    CircuitBreakerOpen {
        /// Resource key.
        resource_key: ResourceKey,
        /// Operation name (`create` or `recycle`).
        operation: &'static str,
        /// Suggested delay before a retry probe.
        retry_after: Duration,
    },
    /// Circuit breaker closed after recovery.
    CircuitBreakerClosed {
        /// Resource key.
        resource_key: ResourceKey,
        /// Operation name (`create` or `recycle`).
        operation: &'static str,
    },
    /// A resource was placed in quarantine.
    Quarantined {
        /// Resource key.
        resource_key: ResourceKey,
        /// Reason for quarantine.
        reason: String,
        /// Structured trigger metadata for observability pipelines.
        trigger: QuarantineTrigger,
        /// Health state before quarantine propagation.
        from_health: HealthState,
        /// Health state after quarantine propagation.
        to_health: HealthState,
    },
    /// A resource was released from quarantine.
    QuarantineReleased {
        /// Resource key.
        resource_key: ResourceKey,
        /// Number of recovery attempts before release.
        recovery_attempts: u32,
    },
    /// A resource's configuration was reloaded (hot-reload).
    ConfigReloaded {
        /// Resource key.
        resource_key: ResourceKey,
        /// Scope after reload.
        scope: Scope,
    },
    /// A config reload attempt was rejected by validation/initialization guardrails.
    ConfigReloadRejected {
        /// Resource key.
        resource_key: ResourceKey,
        /// Validation/init error message.
        error: String,
        /// Whether there was an already-registered pool kept intact.
        had_existing_pool: bool,
    },
    /// An error occurred during a resource operation.
    Error {
        /// Resource key.
        resource_key: ResourceKey,
        /// Error message.
        error: String,
    },
    /// A resource pool's credential was rotated and re-authorization was applied.
    CredentialRotated {
        /// Resource key of the affected pool.
        resource_key: ResourceKey,
        /// The protocol type that was rotated.
        credential_key: nebula_core::CredentialKey,
        /// Strategy that was applied.
        strategy: String,
    },
}

impl ResourceEvent {
    fn key(&self) -> &ResourceKey {
        match self {
            Self::Created { resource_key, .. }
            | Self::Acquired { resource_key, .. }
            | Self::Released { resource_key, .. }
            | Self::HealthChanged { resource_key, .. }
            | Self::PoolExhausted { resource_key, .. }
            | Self::CleanedUp { resource_key, .. }
            | Self::CircuitBreakerOpen { resource_key, .. }
            | Self::CircuitBreakerClosed { resource_key, .. }
            | Self::Quarantined { resource_key, .. }
            | Self::QuarantineReleased { resource_key, .. }
            | Self::ConfigReloaded { resource_key, .. }
            | Self::ConfigReloadRejected { resource_key, .. }
            | Self::Error { resource_key, .. }
            | Self::CredentialRotated { resource_key, .. } => resource_key,
        }
    }
}

impl ScopedEvent for ResourceEvent {
    fn workflow_id(&self) -> Option<&str> {
        let scope = match self {
            Self::Created { scope, .. } | Self::ConfigReloaded { scope, .. } => scope,
            _ => return None,
        };

        match scope {
            Scope::Workflow { workflow_id, .. } => Some(workflow_id),
            Scope::Execution {
                workflow_id: Some(workflow_id),
                ..
            }
            | Scope::Action {
                workflow_id: Some(workflow_id),
                ..
            } => Some(workflow_id),
            _ => None,
        }
    }

    fn execution_id(&self) -> Option<&str> {
        let scope = match self {
            Self::Created { scope, .. } | Self::ConfigReloaded { scope, .. } => scope,
            _ => return None,
        };

        match scope {
            Scope::Execution { execution_id, .. } => Some(execution_id),
            Scope::Action {
                execution_id: Some(execution_id),
                ..
            } => Some(execution_id),
            _ => None,
        }
    }

    fn resource_id(&self) -> Option<&str> {
        Some(self.key().as_ref())
    }
}

/// Structured trigger information for quarantine transitions.
#[derive(Debug, Clone)]
pub enum QuarantineTrigger {
    /// Quarantine was triggered after health-check threshold breach.
    HealthThresholdExceeded {
        /// Consecutive health-check failures observed.
        consecutive_failures: u32,
    },
    /// Quarantine was triggered manually by operator/automation.
    Manual {
        /// Human-readable reason.
        reason: String,
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
    /// The instance was evicted due to credential rotation.
    CredentialRotated,
    /// Recycling the instance failed.
    RecycleFailed,
}

// ---------------------------------------------------------------------------
// Tests (eventbus-dependent tests only with "events" feature)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn default_creates_bus_with_1024_buffer() {
        let bus = EventBus::default();
        let _sub = bus.subscribe();
        assert_eq!(bus.buffer_size(), 1024);
    }

    #[test]
    fn emit_without_subscribers_does_not_panic() {
        let bus = EventBus::new(16);
        let key = ResourceKey::try_from("test").expect("valid resource key");
        bus.emit(ResourceEvent::Created {
            resource_key: key,
            scope: Scope::Global,
        });
        let stats = bus.stats();
        assert_eq!(stats.dropped_count, 1);
    }

    #[tokio::test]
    async fn subscriber_receives_emitted_event() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe();

        let key = ResourceKey::try_from("db").expect("valid resource key");
        bus.emit(ResourceEvent::Created {
            resource_key: key.clone(),
            scope: Scope::Global,
        });

        let event = sub.recv().await.expect("should receive event");
        match event {
            ResourceEvent::Created { resource_key, .. } => {
                assert_eq!(resource_key, key);
            }
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

        let key = ResourceKey::try_from("db").expect("valid resource key");
        bus.emit(ResourceEvent::ConfigReloaded {
            resource_key: key,
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

        let key = ResourceKey::try_from("redis").expect("valid resource key");
        bus.emit(ResourceEvent::Error {
            resource_key: key,
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
        let key = ResourceKey::try_from("x").expect("valid resource key");
        bus.emit(ResourceEvent::Created {
            resource_key: key,
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

        let key = ResourceKey::try_from("x").expect("valid resource key");
        bus.emit(ResourceEvent::Created {
            resource_key: key,
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

        let key = ResourceKey::try_from("y").expect("valid resource key");
        bus.emit_async(ResourceEvent::Created {
            resource_key: key,
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

        let key = ResourceKey::try_from("z").expect("valid resource key");
        bus.emit_async(ResourceEvent::Created {
            resource_key: key,
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

    #[tokio::test]
    async fn scoped_subscription_filters_by_resource_key() {
        let bus = EventBus::new(16);
        let mut sub = bus.subscribe_scoped(SubscriptionScope::resource("db.main"));

        let _ = bus.emit(ResourceEvent::Error {
            resource_key: ResourceKey::try_from("cache.redis").expect("valid resource key"),
            error: "miss".to_string(),
        });
        let _ = bus.emit(ResourceEvent::Error {
            resource_key: ResourceKey::try_from("db.main").expect("valid resource key"),
            error: "timeout".to_string(),
        });

        let event = sub.recv().await.expect("should receive scoped event");
        assert!(matches!(
            event,
            ResourceEvent::Error {
                resource_key,
                error
            } if resource_key.as_ref() == "db.main" && error == "timeout"
        ));
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
