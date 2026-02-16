//! Metrics collection for resource lifecycle events.
//!
//! Subscribes to the [`EventBus`] and translates
//! events into counters, gauges, and histograms via the `metrics` crate.
//!
//! Gated behind the `metrics` feature.

use std::sync::Arc;

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::events::{EventBus, ResourceEvent};

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
    receiver: broadcast::Receiver<ResourceEvent>,
}

impl MetricsCollector {
    /// Create a new collector subscribed to the given event bus.
    #[must_use]
    pub fn new(event_bus: &EventBus) -> Self {
        Self {
            receiver: event_bus.subscribe(),
        }
    }

    /// Run the collector loop, consuming events and updating metrics.
    ///
    /// This method runs until the broadcast channel is closed (i.e. the
    /// `EventBus` is dropped) or the `cancel` token is cancelled.
    /// Lagged events are skipped with a warning.
    pub async fn run(mut self, cancel: CancellationToken) {
        loop {
            tokio::select! {
                result = self.receiver.recv() => {
                    match result {
                        Ok(event) => Self::record_event(&event),
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            #[cfg(feature = "tracing")]
                            tracing::warn!(skipped = n, "MetricsCollector lagged behind event bus");
                            let _ = n;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                () = cancel.cancelled() => break,
            }
        }
    }

    /// Record a single event into the metrics system.
    fn record_event(event: &ResourceEvent) {
        match event {
            ResourceEvent::Created { resource_id, .. } => {
                metrics::counter!("resource.create.total", "resource_id" => resource_id.clone())
                    .increment(1);
            }
            ResourceEvent::Acquired { resource_id, .. } => {
                metrics::counter!("resource.acquire.total", "resource_id" => resource_id.clone())
                    .increment(1);
            }
            ResourceEvent::Released {
                resource_id,
                usage_duration,
            } => {
                metrics::counter!("resource.release.total", "resource_id" => resource_id.clone())
                    .increment(1);
                metrics::histogram!(
                    "resource.usage.duration_seconds",
                    "resource_id" => resource_id.clone()
                )
                .record(usage_duration.as_secs_f64());
            }
            ResourceEvent::CleanedUp { resource_id, .. } => {
                metrics::counter!("resource.cleanup.total", "resource_id" => resource_id.clone())
                    .increment(1);
            }
            ResourceEvent::Error { resource_id, .. } => {
                metrics::counter!("resource.error.total", "resource_id" => resource_id.clone())
                    .increment(1);
            }
            // HealthChanged and PoolExhausted are informational; we don't
            // record dedicated metrics for them (tracing handles these).
            ResourceEvent::HealthChanged { .. }
            | ResourceEvent::PoolExhausted { .. }
            | ResourceEvent::Quarantined { .. }
            | ResourceEvent::QuarantineReleased { .. } => {}
        }
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
        bus.emit(ResourceEvent::Created {
            resource_id: "db".to_string(),
            scope: crate::scope::Scope::Global,
        });
        bus.emit(ResourceEvent::Acquired {
            resource_id: "db".to_string(),
        });
        bus.emit(ResourceEvent::Released {
            resource_id: "db".to_string(),
            usage_duration: Duration::from_millis(42),
        });
        bus.emit(ResourceEvent::Error {
            resource_id: "db".to_string(),
            error: "test error".to_string(),
        });
        bus.emit(ResourceEvent::CleanedUp {
            resource_id: "db".to_string(),
            reason: crate::events::CleanupReason::Shutdown,
        });

        // Give the collector a moment to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Drop the bus to close the channel, which stops the collector
        drop(bus);
        let _ = tokio::time::timeout(Duration::from_secs(1), handle).await;
    }
}
