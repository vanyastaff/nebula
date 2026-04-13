//! `RoutingMap` — `(trigger_uuid, nonce) → Arc<ActivationEntry>`
//! lookup backed by `DashMap`.
//!
//! Each entry holds the `TriggerHandler` for the active webhook
//! registration plus the `TriggerContext` template the transport
//! clones on every incoming request. The nonce in the key is a
//! per-activation random string (16-byte hex), so stale external
//! hooks pointing at the same trigger UUID but an old nonce cannot
//! route to a fresh registration.

use std::sync::Arc;

use dashmap::DashMap;
use nebula_action::{TriggerContext, TriggerHandler};
use uuid::Uuid;

/// Composite key used inside the routing map.
pub(crate) type RouteKey = (Uuid, String);

/// Single registered webhook activation.
///
/// Holds the handler pointer and a template [`TriggerContext`] that
/// the transport clones on every request. Cloning gives each
/// dispatch its own independent context without locking — the
/// capability arcs inside (`emitter`, `health`, `webhook`, etc.)
/// share state as designed.
#[derive(Clone)]
pub(crate) struct ActivationEntry {
    pub(crate) handler: Arc<dyn TriggerHandler>,
    pub(crate) ctx: TriggerContext,
}

impl std::fmt::Debug for ActivationEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivationEntry")
            .field("handler_key", &self.handler.metadata().key)
            .field("trigger_id", &self.ctx.trigger_id)
            .field("workflow_id", &self.ctx.workflow_id)
            .finish_non_exhaustive()
    }
}

/// Thread-safe `(uuid, nonce)` → activation lookup table.
#[derive(Debug, Default)]
pub(crate) struct RoutingMap {
    entries: DashMap<RouteKey, Arc<ActivationEntry>>,
}

impl RoutingMap {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    /// Register a new activation. Returns `false` if an entry for
    /// the same `(uuid, nonce)` already exists — in practice this
    /// should never happen because the nonce is freshly generated
    /// per activation, but we reject collisions defensively.
    pub(crate) fn insert(&self, uuid: Uuid, nonce: String, entry: ActivationEntry) -> bool {
        match self.entries.entry((uuid, nonce)) {
            dashmap::Entry::Occupied(_) => false,
            dashmap::Entry::Vacant(v) => {
                v.insert(Arc::new(entry));
                true
            }
        }
    }

    /// Look up an entry by `(uuid, nonce)`. Returns a cheap `Arc`
    /// clone so the transport can drop the DashMap guard before
    /// `await`-ing the handler.
    pub(crate) fn lookup(&self, uuid: &Uuid, nonce: &str) -> Option<Arc<ActivationEntry>> {
        self.entries
            .get(&(*uuid, nonce.to_string()))
            .map(|e| Arc::clone(&*e))
    }

    /// Remove an entry. Called by the transport on trigger
    /// deactivation.
    pub(crate) fn remove(&self, uuid: &Uuid, nonce: &str) -> bool {
        self.entries.remove(&(*uuid, nonce.to_string())).is_some()
    }

    /// Current number of active registrations. Mostly for tests
    /// and observability.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal dummy TriggerHandler for the routing tests so we
    // don't need a real webhook action here.
    struct Noop {
        meta: nebula_action::ActionMetadata,
    }

    #[async_trait::async_trait]
    impl TriggerHandler for Noop {
        fn metadata(&self) -> &nebula_action::ActionMetadata {
            &self.meta
        }
        async fn start(&self, _ctx: &TriggerContext) -> Result<(), nebula_action::ActionError> {
            Ok(())
        }
        async fn stop(&self, _ctx: &TriggerContext) -> Result<(), nebula_action::ActionError> {
            Ok(())
        }
    }

    fn dummy_entry() -> ActivationEntry {
        use tokio_util::sync::CancellationToken;
        let handler: Arc<dyn TriggerHandler> = Arc::new(Noop {
            meta: nebula_action::ActionMetadata::new(
                nebula_core::action_key!("test.routing.noop"),
                "Noop",
                "routing map unit test",
            ),
        });
        let ctx = TriggerContext::new(
            nebula_core::WorkflowId::new(),
            nebula_core::NodeId::new(),
            CancellationToken::new(),
        );
        ActivationEntry { handler, ctx }
    }

    #[test]
    fn insert_lookup_remove_roundtrip() {
        let map = RoutingMap::new();
        let uuid = Uuid::new_v4();
        assert!(map.insert(uuid, "nonce1".into(), dummy_entry()));
        assert_eq!(map.len(), 1);
        assert!(map.lookup(&uuid, "nonce1").is_some());
        assert!(map.lookup(&uuid, "nonce2").is_none());
        assert!(map.remove(&uuid, "nonce1"));
        assert_eq!(map.len(), 0);
        assert!(map.lookup(&uuid, "nonce1").is_none());
    }

    #[test]
    fn insert_rejects_duplicate_key() {
        let map = RoutingMap::new();
        let uuid = Uuid::new_v4();
        assert!(map.insert(uuid, "n".into(), dummy_entry()));
        assert!(!map.insert(uuid, "n".into(), dummy_entry()));
    }

    #[test]
    fn remove_non_existent_returns_false() {
        let map = RoutingMap::new();
        assert!(!map.remove(&Uuid::new_v4(), "whatever"));
    }
}
