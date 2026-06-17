//! `TriggerLifecycleBus` — event bus for webhook activation lifecycle events.
//!
//! Operator changes to webhook activations (create / update / delete) can be
//! published as events on this bus. Future subscribers reflect the change
//! without reloading the entire registry.
//!
//! The A-world slug subscriber (`TriggerLifecycleSubscriber`) was retired in
//! ADR-0096 commit 3 alongside the slug URL surface. The B-world subscriber
//! (U-D1.4b — durable emitter install) is the planned successor.
//!
//! # Current state
//!
//! The bus is scaffolded here so `AppState::trigger_lifecycle_bus` compiles.
//! Producers (storage CRUD callsites that `emit()`) are intentionally not yet
//! wired — this is per webhook activation §"Out of scope".

use std::sync::Arc;

use nebula_eventbus::EventBus;

/// Placeholder event type for webhook activation lifecycle changes.
///
/// Carries no payload until B-world subscriber (U-D1.4b) lands — the variant
/// set will grow when the producer side is wired.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TriggerLifecycleEvent {
    /// Placeholder — reserved for future B-world lifecycle events.
    Reserved,
}

/// `EventBus` typed for [`TriggerLifecycleEvent`].
pub type TriggerLifecycleEventBus = EventBus<TriggerLifecycleEvent>;

/// `Send + Sync + 'static` wrapper around the bus, intended to live on
/// `AppState`. Newtype rather than re-exporting the alias so future
/// behaviour (per-tenant routing, dropped-event metrics) lands here
/// without churning every consumer.
#[derive(Clone, Debug)]
pub struct TriggerLifecycleBus {
    inner: Arc<TriggerLifecycleEventBus>,
}

impl TriggerLifecycleBus {
    /// Construct a fresh bus with the given buffer capacity. Default
    /// callers should use 256.
    #[must_use]
    pub fn new(buffer: usize) -> Self {
        Self {
            inner: Arc::new(TriggerLifecycleEventBus::new(buffer)),
        }
    }

    /// Wrap an existing bus.
    #[must_use]
    pub fn from_arc(inner: Arc<TriggerLifecycleEventBus>) -> Self {
        Self { inner }
    }

    /// Publish an event. Non-blocking; backpressure follows the bus
    /// policy (defaults to `DropOldest`).
    pub fn emit(&self, event: TriggerLifecycleEvent) {
        let _ = self.inner.emit(event);
    }

    /// Borrow the underlying bus.
    #[must_use]
    pub fn bus(&self) -> Arc<TriggerLifecycleEventBus> {
        Arc::clone(&self.inner)
    }
}

impl Default for TriggerLifecycleBus {
    fn default() -> Self {
        Self::new(256)
    }
}
