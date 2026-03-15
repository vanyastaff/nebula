//! Metrics collection for resource lifecycle events.
//!
//! Subscribes to the [`EventBus`] and translates events into counters, gauges,
//! and histograms via the `metrics` crate. Metric names follow the
//! `nebula_resource_*` convention from `nebula-metrics`.
//!
//! Gated behind the `metrics` feature.

use std::sync::{Arc, LazyLock};

use dashmap::DashSet;
use nebula_core::ResourceKey;

use nebula_metrics::naming::{
    NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_CIRCUIT_BREAKER_CLOSED_TOTAL, NEBULA_RESOURCE_CIRCUIT_BREAKER_OPENED_TOTAL,
    NEBULA_RESOURCE_CLEANUP_TOTAL, NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
    NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
};
use tokio_util::sync::CancellationToken;

use crate::events::{EventBus, EventSubscriber, ResourceEvent};

/// Background metrics collector that subscribes to an [`EventBus`]
/// and records counters/histograms via the `metrics` crate.
///
/// # Usage
///
/// ```rust,ignore
/// let event_bus = Arc::new(EventBus::default());
/// let collector = MetricsCollector::new(&event_bus);
/// let cancel = CancellationToken::new();
/// tokio::spawn(collector.run(cancel));
/// ```
pub struct MetricsCollector {
    subscriber: EventSubscriber<ResourceEvent>,
}

const MAX_RESOURCE_LABEL_CARDINALITY: usize = 128;
static RESOURCE_LABELS: LazyLock<DashSet<String>> = LazyLock::new(DashSet::new);

impl MetricsCollector {
    /// Create a new collector subscribed to the given event bus.
    #[must_use]
    pub fn new(event_bus: &EventBus) -> Self {
        Self {
            subscriber: event_bus.subscribe(),
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
                        Some(e) => Self::record_event(&e),
                        None => break,
                    }
                }
                () = cancel.cancelled() => break,
            }
        }
    }

    /// Record a single event into the metrics system.
    fn record_event(event: &ResourceEvent) {
        match event {
            ResourceEvent::Created { resource_key, .. } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(NEBULA_RESOURCE_CREATE_TOTAL, "resource_id" => id).increment(1);
            }
            ResourceEvent::Acquired {
                resource_key,
                wait_duration,
            } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(NEBULA_RESOURCE_ACQUIRE_TOTAL, "resource_id" => id.clone())
                    .increment(1);
                metrics::histogram!(
                    NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
                    "resource_id" => id
                )
                .record(wait_duration.as_secs_f64());
            }
            ResourceEvent::Released {
                resource_key,
                usage_duration,
            } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(NEBULA_RESOURCE_RELEASE_TOTAL, "resource_id" => id.clone())
                    .increment(1);
                metrics::histogram!(
                    NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
                    "resource_id" => id
                )
                .record(usage_duration.as_secs_f64());
            }
            ResourceEvent::CleanedUp { resource_key, .. } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(NEBULA_RESOURCE_CLEANUP_TOTAL, "resource_id" => id).increment(1);
            }
            ResourceEvent::Error { resource_key, .. } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(NEBULA_RESOURCE_ERROR_TOTAL, "resource_id" => id).increment(1);
            }
            ResourceEvent::HealthChanged {
                resource_key, to, ..
            } => {
                let id = Self::resource_label(resource_key);
                // Map health state to a numeric gauge: 1.0 = healthy, 0.5 = degraded/unknown,
                // 0.0 = unhealthy. This lets alerting thresholds use simple numeric comparisons.
                let score = match to {
                    crate::health::HealthState::Healthy => 1.0,
                    crate::health::HealthState::Degraded { .. } => 0.5,
                    crate::health::HealthState::Unhealthy { .. } => 0.0,
                    crate::health::HealthState::Unknown => 0.5,
                };
                metrics::gauge!(
                    NEBULA_RESOURCE_HEALTH_STATE,
                    "resource_id" => id
                )
                .set(score);
            }
            ResourceEvent::PoolExhausted {
                resource_key,
                waiters,
            } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(
                    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL,
                    "resource_id" => id.clone()
                )
                .increment(1);
                metrics::gauge!(
                    NEBULA_RESOURCE_POOL_WAITERS,
                    "resource_id" => id
                )
                .set(*waiters as f64);
            }
            ResourceEvent::Quarantined { resource_key, .. } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(
                    NEBULA_RESOURCE_QUARANTINE_TOTAL,
                    "resource_id" => id
                )
                .increment(1);
            }
            ResourceEvent::QuarantineReleased { resource_key, .. } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(
                    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL,
                    "resource_id" => id
                )
                .increment(1);
            }
            ResourceEvent::ConfigReloaded { resource_key, .. } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(
                    NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
                    "resource_id" => id
                )
                .increment(1);
            }
            ResourceEvent::ConfigReloadRejected { resource_key, .. } => {
                // A rejected reload is an error condition — count it alongside other errors
                // so dashboards surface it without needing a separate panel.
                let id = Self::resource_label(resource_key);
                metrics::counter!(NEBULA_RESOURCE_ERROR_TOTAL, "resource_id" => id).increment(1);
            }
            ResourceEvent::CredentialRotated { resource_key, .. } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(
                    NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
                    "resource_id" => id
                )
                .increment(1);
            }
            ResourceEvent::CircuitBreakerOpen {
                resource_key,
                operation,
                ..
            } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(
                    NEBULA_RESOURCE_CIRCUIT_BREAKER_OPENED_TOTAL,
                    "resource_id" => id,
                    "operation" => *operation,
                )
                .increment(1);
            }
            ResourceEvent::CircuitBreakerClosed {
                resource_key,
                operation,
            } => {
                let id = Self::resource_label(resource_key);
                metrics::counter!(
                    NEBULA_RESOURCE_CIRCUIT_BREAKER_CLOSED_TOTAL,
                    "resource_id" => id,
                    "operation" => *operation,
                )
                .increment(1);
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
    fn resource_label(resource_key: &ResourceKey) -> String {
        let raw = resource_key.as_ref();
        // Truncate long keys and mark them so they are distinguishable from the
        // original full-length key in dashboards.
        let normalized = if raw.len() > 64 {
            format!("{}~", &raw[..63])
        } else {
            raw.to_string()
        };

        // Fast path: label already registered.
        if RESOURCE_LABELS.contains(&normalized) {
            return normalized;
        }

        // Guard against unbounded cardinality growth.
        if RESOURCE_LABELS.len() >= MAX_RESOURCE_LABEL_CARDINALITY {
            return "__other".to_string();
        }

        RESOURCE_LABELS.insert(normalized.clone());
        normalized
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
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let collector = MetricsCollector::new(event_bus);
    tokio::spawn(collector.run(cancel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use nebula_core::ResourceKey;
    use std::time::Duration;

    #[tokio::test]
    async fn collector_processes_events_without_panic() {
        // We cannot easily inspect metrics crate internals, but we can
        // verify the collector runs and processes events without errors.
        let bus = Arc::new(EventBus::new(64));
        let collector = MetricsCollector::new(&bus);
        let cancel = CancellationToken::new();

        let handle = tokio::spawn(collector.run(cancel));

        // Emit a variety of events
        let key = nebula_core::ResourceKey::try_from("db").expect("valid resource key");
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

        // Drop the bus to close the channel, which stops the collector
        drop(bus);
        let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
    }

    #[test]
    fn resource_label_caps_cardinality() {
        RESOURCE_LABELS.clear();
        while RESOURCE_LABELS.len() < MAX_RESOURCE_LABEL_CARDINALITY {
            let seed = format!("seed-{}", RESOURCE_LABELS.len());
            RESOURCE_LABELS.insert(seed);
        }

        let extra_key = ResourceKey::try_from("r-overflow").expect("valid key");
        let overflow_label = MetricsCollector::resource_label(&extra_key);
        assert_eq!(overflow_label, "__other");
    }
}
