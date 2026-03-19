//! Metrics collection for resource lifecycle events.
//!
//! Subscribes to the [`EventBus`] and translates events into counters, gauges,
//! and histograms via the [`MetricsRegistry`]. Metric names follow the
//! `nebula_resource_*` convention from `nebula-metrics`.

use std::sync::Arc;

use dashmap::DashSet;
use nebula_core::ResourceKey;
use nebula_metrics::naming::{
    RESOURCE_ACQUIRE, RESOURCE_ACQUIRE_WAIT_DURATION, RESOURCE_CIRCUIT_BREAKER_CLOSED,
    RESOURCE_CIRCUIT_BREAKER_OPENED, RESOURCE_CLEANUP, RESOURCE_CONFIG_RELOADED, RESOURCE_CREATE,
    RESOURCE_ERROR, RESOURCE_HEALTH_STATE, RESOURCE_POOL_EXHAUSTED, RESOURCE_POOL_WAITERS,
    RESOURCE_QUARANTINE, RESOURCE_QUARANTINE_RELEASED, RESOURCE_RELEASE, RESOURCE_USAGE_DURATION,
};
use nebula_telemetry::labels::LabelSet;
use nebula_telemetry::metrics::MetricsRegistry;
use tokio_util::sync::CancellationToken;

use crate::events::{EventBus, EventSubscriber, ResourceEvent};

/// Maximum number of distinct `resource_id` label values before overflow
/// labels are bucketed into `"__other"`.
const MAX_RESOURCE_LABEL_CARDINALITY: usize = 128;

/// Background metrics collector that subscribes to an [`EventBus`]
/// and records counters/gauges/histograms into a [`MetricsRegistry`].
///
/// # Usage
///
/// ```rust,ignore
/// let registry = Arc::new(MetricsRegistry::new());
/// let event_bus = Arc::new(EventBus::default());
/// let collector = MetricsCollector::new(&event_bus, Arc::clone(&registry));
/// let cancel = CancellationToken::new();
/// tokio::spawn(collector.run(cancel));
/// ```
pub struct MetricsCollector {
    /// Event subscriber for resource lifecycle events.
    subscriber: EventSubscriber<ResourceEvent>,
    /// Shared metrics registry for recording counters/gauges/histograms.
    registry: Arc<MetricsRegistry>,
    /// Per-instance cardinality guard (replaces the former global `LazyLock`).
    seen_labels: DashSet<String>,
}

impl MetricsCollector {
    /// Create a new collector subscribed to the given event bus, recording
    /// into the provided [`MetricsRegistry`].
    #[must_use]
    pub fn new(event_bus: &EventBus, registry: Arc<MetricsRegistry>) -> Self {
        Self {
            subscriber: event_bus.subscribe(),
            registry,
            seen_labels: DashSet::new(),
        }
    }

    /// Run the collector loop, consuming events and updating metrics.
    ///
    /// This method runs until the event bus is closed (i.e. the
    /// `EventBus` is dropped) or the `cancel` token is cancelled.
    /// Lagged events are skipped internally by the eventbus subscriber.
    pub async fn run(mut self, cancel: CancellationToken) {
        loop {
            tokio::select! {
                event = self.subscriber.recv() => {
                    match event {
                        Some(e) => self.record_event(&e),
                        None => break,
                    }
                }
                () = cancel.cancelled() => break,
            }
        }
    }

    /// Record a single event into the metrics registry.
    fn record_event(&self, event: &ResourceEvent) {
        match event {
            ResourceEvent::Created { resource_key, .. } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_CREATE.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::Acquired {
                resource_key,
                wait_duration,
            } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_ACQUIRE.as_str(), &labels)
                    .inc();
                self.registry
                    .histogram_labeled(RESOURCE_ACQUIRE_WAIT_DURATION.as_str(), &labels)
                    .observe(wait_duration.as_secs_f64());
            }
            ResourceEvent::Released {
                resource_key,
                usage_duration,
            } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_RELEASE.as_str(), &labels)
                    .inc();
                self.registry
                    .histogram_labeled(RESOURCE_USAGE_DURATION.as_str(), &labels)
                    .observe(usage_duration.as_secs_f64());
            }
            ResourceEvent::CleanedUp { resource_key, .. } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_CLEANUP.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::Error { resource_key, .. } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_ERROR.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::HealthChanged {
                resource_key, to, ..
            } => {
                let labels = self.resource_labels(resource_key);
                // Map health state to an integer gauge:
                //   100 = healthy, 50 = degraded/unknown, 0 = unhealthy.
                // Integer values because MetricsRegistry gauges are i64.
                let score: i64 = match to {
                    crate::health::HealthState::Healthy => 100,
                    crate::health::HealthState::Degraded { .. } => 50,
                    crate::health::HealthState::Unhealthy { .. } => 0,
                    crate::health::HealthState::Unknown => 50,
                };
                self.registry
                    .gauge_labeled(RESOURCE_HEALTH_STATE.as_str(), &labels)
                    .set(score);
            }
            ResourceEvent::PoolExhausted {
                resource_key,
                waiters,
            } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_POOL_EXHAUSTED.as_str(), &labels)
                    .inc();
                #[allow(clippy::cast_possible_wrap)]
                // Reason: waiter counts are small non-negative values; i64 wrapping is unreachable.
                self.registry
                    .gauge_labeled(RESOURCE_POOL_WAITERS.as_str(), &labels)
                    .set(*waiters as i64);
            }
            ResourceEvent::Quarantined { resource_key, .. } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_QUARANTINE.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::QuarantineReleased { resource_key, .. } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_QUARANTINE_RELEASED.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::ConfigReloaded { resource_key, .. } => {
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_CONFIG_RELOADED.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::ConfigReloadRejected { resource_key, .. } => {
                // A rejected reload is an error condition — count it alongside other errors
                // so dashboards surface it without needing a separate panel.
                let labels = self.resource_labels(resource_key);
                self.registry
                    .counter_labeled(RESOURCE_ERROR.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::CircuitBreakerOpen {
                resource_key,
                operation,
                ..
            } => {
                let id = self.resource_label(resource_key);
                let labels = self
                    .registry
                    .interner()
                    .label_set(&[("resource_id", &*id), ("operation", operation)]);
                self.registry
                    .counter_labeled(RESOURCE_CIRCUIT_BREAKER_OPENED.as_str(), &labels)
                    .inc();
            }
            ResourceEvent::CircuitBreakerClosed {
                resource_key,
                operation,
            } => {
                let id = self.resource_label(resource_key);
                let labels = self
                    .registry
                    .interner()
                    .label_set(&[("resource_id", &*id), ("operation", operation)]);
                self.registry
                    .counter_labeled(RESOURCE_CIRCUIT_BREAKER_CLOSED.as_str(), &labels)
                    .inc();
            }
        }
    }

    /// Produce a stable metric label string for a resource key.
    ///
    /// Labels are truncated at 64 characters (with a trailing `~` marker) to keep
    /// cardinality manageable in time-series databases. Once
    /// [`MAX_RESOURCE_LABEL_CARDINALITY`] distinct labels have been seen, any new
    /// resource key maps to the sentinel `"__other"` label instead of creating a
    /// new series. This prevents a cardinality explosion in deployments that
    /// dynamically register many resource keys.
    fn resource_label(&self, resource_key: &ResourceKey) -> String {
        let raw: &str = resource_key;
        // Truncate long keys and mark them so they are distinguishable from the
        // original full-length key in dashboards.
        let normalized = if raw.len() > 64 {
            format!("{}~", &raw[..63])
        } else {
            raw.to_string()
        };

        // Fast path: label already registered.
        if self.seen_labels.contains(&normalized) {
            return normalized;
        }

        // Guard against unbounded cardinality growth.
        if self.seen_labels.len() >= MAX_RESOURCE_LABEL_CARDINALITY {
            return "__other".to_string();
        }

        self.seen_labels.insert(normalized.clone());
        normalized
    }

    /// Build a [`LabelSet`] containing only the `resource_id` label.
    fn resource_labels(&self, resource_key: &ResourceKey) -> LabelSet {
        let id = self.resource_label(resource_key);
        self.registry.interner().label_set(&[("resource_id", &*id)])
    }
}

impl std::fmt::Debug for MetricsCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsCollector").finish()
    }
}

/// Create a [`MetricsCollector`] and spawn it as a background task.
///
/// The task stops when `cancel` is cancelled or the `EventBus` is dropped.
/// Returns the `JoinHandle` so the caller can await or abort the task.
pub fn spawn_metrics_collector(
    event_bus: &Arc<EventBus>,
    registry: Arc<MetricsRegistry>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let collector = MetricsCollector::new(event_bus, Arc::clone(&registry));
    tokio::spawn(collector.run(cancel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use nebula_core::resource_key;
    use std::time::Duration;

    #[tokio::test]
    async fn collector_processes_events_without_panic() {
        let registry = Arc::new(MetricsRegistry::new());
        let bus = Arc::new(EventBus::new(64));
        let collector = MetricsCollector::new(&bus, Arc::clone(&registry));
        let cancel = CancellationToken::new();

        let handle = tokio::spawn(collector.run(cancel));

        // Emit a variety of events
        let key = resource_key!("db");
        bus.emit(ResourceEvent::Created {
            resource_key: key.clone(),
            scope: crate::scope::Scope::Global,
        });
        bus.emit(ResourceEvent::Acquired {
            resource_key: key.clone(),
            wait_duration: Duration::from_millis(5),
        });
        bus.emit(ResourceEvent::Released {
            resource_key: key.clone(),
            usage_duration: Duration::from_millis(42),
        });
        bus.emit(ResourceEvent::Error {
            resource_key: key.clone(),
            error: "test error".to_string(),
        });
        bus.emit(ResourceEvent::CleanedUp {
            resource_key: key,
            reason: crate::events::CleanupReason::Shutdown,
        });

        // Give the collector a moment to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify counters are recorded in the registry
        let counters = registry.snapshot_counters();
        let create_count: u64 = counters
            .iter()
            .filter(|(k, _)| registry.interner().resolve(k.name) == RESOURCE_CREATE.as_str())
            .map(|(_, c)| c.get())
            .sum();
        assert_eq!(create_count, 1);

        // Drop the bus to close the channel, which stops the collector
        drop(bus);
        let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
    }

    #[test]
    fn resource_label_caps_cardinality() {
        let registry = Arc::new(MetricsRegistry::new());
        let bus = EventBus::new(16);
        let collector = MetricsCollector::new(&bus, registry);

        // Fill up to the cardinality limit
        for i in 0..MAX_RESOURCE_LABEL_CARDINALITY {
            let key = ResourceKey::new(&format!("seed-{i}")).unwrap();
            let label = collector.resource_label(&key);
            assert_ne!(label, "__other", "should not overflow at index {i}");
        }

        // Next label should overflow
        let extra_key = resource_key!("r-overflow");
        let overflow_label = collector.resource_label(&extra_key);
        assert_eq!(overflow_label, "__other");
    }
}
