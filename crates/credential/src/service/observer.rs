//! Non-optional observability seam. Closes credential secrecy/§3.5: emission
//! sits on the single facade code path, so "never wired" is
//! unrepresentable. `CredentialObserver` is object-safe by design
//! (`Arc<dyn CredentialObserver>`).

use std::sync::Arc;

use nebula_core::accessor::MetricsEmitter;
use nebula_eventbus::EventBus;

use crate::metrics::CredentialMetrics;
use crate::provider::LeaseEvent;
use crate::{CredentialEvent, CredentialId};

/// Observability hooks the facade calls on every lifecycle transition.
/// Object-safe (no RPITIT / generics) so it can be `Arc<dyn …>`.
pub trait CredentialObserver: Send + Sync {
    /// Event bus the internally-built `CredentialResolver` is wired to
    /// (`.with_event_bus`). Non-optional — the resolver always gets a
    /// real bus.
    fn event_bus(&self) -> Arc<EventBus<CredentialEvent>>;
    /// Optional lease event bus handed to `LeaseLifecycle::spawn`.
    fn lease_bus(&self) -> Option<Arc<EventBus<LeaseEvent>>>;
    /// Optional metrics emitter handed to `LeaseLifecycle::spawn` and
    /// used by the facade for resolve/refresh/test counters.
    fn metrics(&self) -> Option<Arc<dyn MetricsEmitter>>;
    /// Called after a successful resolve.
    fn on_resolve(&self, credential_id: &CredentialId);
    /// Called after a successful refresh.
    fn on_refresh(&self, credential_id: &CredentialId);
    /// Called after interactive credential material was replaced.
    fn on_material_replaced(
        &self,
        scope: &crate::TenantScope,
        credential_id: &CredentialId,
        credential_key: &nebula_core::CredentialKey,
    ) {
        let _ = self.event_bus().emit(CredentialEvent::MaterialReplaced {
            credential_id: *credential_id,
            scope: scope.durable_owner_scope(),
            credential_key: credential_key.clone(),
        });
    }
    /// Called after a successful revoke.
    fn on_revoke(&self, credential_id: &CredentialId);
}

/// Silent observer. Must be chosen *explicitly* at the composition root
/// (tests) — never a default that hides missing wiring.
///
/// Holds one cached event bus so [`event_bus`](CredentialObserver::event_bus)
/// is idempotent: the resolver wired at `build()` and anything that later
/// queries the observer share the *same* bus instead of each call minting
/// a fresh, disconnected `EventBus`.
#[derive(Debug, Clone)]
pub struct NoopObserver {
    bus: Arc<EventBus<CredentialEvent>>,
}

impl NoopObserver {
    /// Construct a silent observer with its single cached event bus.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Capacity 1: the noop observer never emits, the bus exists
            // only so the resolver has a real (if unused) handle.
            bus: Arc::new(EventBus::new(1)),
        }
    }
}

impl Default for NoopObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialObserver for NoopObserver {
    fn event_bus(&self) -> Arc<EventBus<CredentialEvent>> {
        Arc::clone(&self.bus)
    }
    fn lease_bus(&self) -> Option<Arc<EventBus<LeaseEvent>>> {
        None
    }
    fn metrics(&self) -> Option<Arc<dyn MetricsEmitter>> {
        None
    }
    fn on_resolve(&self, _credential_id: &CredentialId) {}
    fn on_refresh(&self, _credential_id: &CredentialId) {}
    fn on_material_replaced(
        &self,
        _scope: &crate::TenantScope,
        _credential_id: &CredentialId,
        _credential_key: &nebula_core::CredentialKey,
    ) {
    }
    fn on_revoke(&self, _credential_id: &CredentialId) {}
}

/// Production observer: emits `CredentialEvent` to an `EventBus`,
/// increments `CredentialMetrics` counters via the supplied emitter.
pub struct EventMetricObserver {
    events: Arc<EventBus<CredentialEvent>>,
    leases: Arc<EventBus<LeaseEvent>>,
    metrics: Option<Arc<dyn MetricsEmitter>>,
}

impl EventMetricObserver {
    /// `buffer` is the per-bus capacity.
    #[must_use]
    pub fn new(buffer: usize) -> Self {
        Self {
            events: Arc::new(EventBus::new(buffer)),
            leases: Arc::new(EventBus::new(buffer)),
            metrics: None,
        }
    }

    /// Attach a metrics emitter (counters for resolve/refresh).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metrics(mut self, emitter: Arc<dyn MetricsEmitter>) -> Self {
        self.metrics = Some(emitter);
        self
    }

    fn count(&self, name: &str, outcome: &str) {
        if let Some(m) = &self.metrics {
            m.counter(name, 1, &[("outcome", outcome)]);
        }
    }
}

impl CredentialObserver for EventMetricObserver {
    fn event_bus(&self) -> Arc<EventBus<CredentialEvent>> {
        Arc::clone(&self.events)
    }
    fn lease_bus(&self) -> Option<Arc<EventBus<LeaseEvent>>> {
        Some(Arc::clone(&self.leases))
    }
    fn metrics(&self) -> Option<Arc<dyn MetricsEmitter>> {
        self.metrics.clone()
    }
    fn on_resolve(&self, _credential_id: &CredentialId) {
        self.count(CredentialMetrics::RESOLVE_TOTAL, "ok");
    }
    fn on_refresh(&self, credential_id: &CredentialId) {
        let _ = self.events.emit(CredentialEvent::Refreshed {
            credential_id: *credential_id,
        });
        self.count(CredentialMetrics::REFRESH_TOTAL, "ok");
    }
    fn on_revoke(&self, credential_id: &CredentialId) {
        let _ = self.events.emit(CredentialEvent::Revoked {
            credential_id: *credential_id,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{CredentialObserver, EventMetricObserver, NoopObserver};
    use crate::{CredentialId, CredentialMetrics, TenantScope};
    use nebula_core::accessor::MetricsEmitter;

    #[derive(Default)]
    struct RecordingMetrics(Mutex<Vec<String>>);

    impl MetricsEmitter for RecordingMetrics {
        fn counter(&self, name: &str, _value: u64, _labels: &[(&str, &str)]) {
            self.0.lock().expect("metrics lock").push(name.to_owned());
        }

        fn gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}

        fn histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
    }

    #[test]
    fn noop_observer_is_object_safe_and_silent() {
        let obs: Arc<dyn CredentialObserver> = Arc::new(NoopObserver::new());
        obs.on_revoke(&CredentialId::new());
        assert!(obs.lease_bus().is_none());
        assert!(obs.metrics().is_none());
        // `event_bus()` is idempotent — same cached bus every call.
        assert!(Arc::ptr_eq(&obs.event_bus(), &obs.event_bus()));
    }

    #[tokio::test]
    async fn event_metric_observer_emits_on_event_bus() {
        let obs = EventMetricObserver::new(8);
        let mut sub = obs.event_bus().subscribe();
        obs.on_refresh(&CredentialId::new());
        let ev = sub.try_recv().expect("event emitted");
        assert!(matches!(ev, crate::CredentialEvent::Refreshed { .. }));
    }

    #[tokio::test]
    async fn material_replacement_emits_without_counting_a_refresh() {
        let metrics = Arc::new(RecordingMetrics::default());
        let observer = EventMetricObserver::new(8).with_metrics(metrics.clone());
        let mut subscriber = observer.event_bus().subscribe();
        let id = CredentialId::new();
        let key = "oauth".parse().expect("valid credential key");

        observer.on_material_replaced(&TenantScope::new("org", "workspace"), &id, &key);

        assert!(matches!(
            subscriber.try_recv().expect("replacement event emitted"),
            crate::CredentialEvent::MaterialReplaced { credential_id, .. }
                if credential_id == id
        ));
        assert!(
            !metrics
                .0
                .lock()
                .expect("metrics lock")
                .iter()
                .any(|name| name == CredentialMetrics::REFRESH_TOTAL),
            "reauthorization must not inflate refresh metrics"
        );
    }
}
