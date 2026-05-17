//! Converged routing map keyed on [`WebhookKey`].
//!
//! Both programmatic `(uuid, nonce)` and slug `(org, workspace, slug)`
//! activations live in the same `DashMap`. The transport's
//! `dispatch_inner` performs a single lookup and runs the same
//! signature → replay → rate-limit → pre-handle → handle pipeline
//! regardless of how the key was constructed.

use std::sync::Arc;

use dashmap::DashMap;
use nebula_action::{TriggerHandler, TriggerRuntimeContext, WebhookConfig};

use super::key::WebhookKey;

/// Single registered webhook activation.
///
/// Holds the handler pointer, a template [`TriggerRuntimeContext`]
/// that the transport clones on every request, and the
/// [`WebhookConfig`] enforced before dispatch (per ADR-0022). Cloning
/// the context gives each dispatch its own independent context
/// without locking — the capability arcs inside (`emitter`, `health`,
/// `webhook`, etc.) share state as designed.
///
/// The config is read once from the typed
/// [`nebula_action::WebhookAction`] at activation time by whoever
/// owns the typed handler (runtime registry or test harness) and
/// handed to the transport alongside the handler. It is *not* read
/// through the dyn `TriggerHandler` surface — webhook-specific
/// configuration does not belong on the base trigger contract.
#[derive(Clone)]
pub(crate) struct ActivationEntry {
    pub(crate) handler: Arc<dyn TriggerHandler>,
    pub(crate) ctx: TriggerRuntimeContext,
    pub(crate) config: WebhookConfig,
}

impl std::fmt::Debug for ActivationEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use nebula_core::Context;
        f.debug_struct("ActivationEntry")
            .field("handler_key", &self.handler.metadata().base.key)
            .field("trigger_id", &self.ctx.trigger_id())
            .field("workflow_id", &self.ctx.scope().workflow_id)
            .finish_non_exhaustive()
    }
}

/// Thread-safe [`WebhookKey`] → activation lookup table.
#[derive(Debug, Default)]
pub(crate) struct RoutingMap {
    entries: DashMap<WebhookKey, Arc<ActivationEntry>>,
}

impl RoutingMap {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    /// Register a new activation. Returns `false` if an entry for
    /// the same key already exists.
    pub(crate) fn insert(&self, key: WebhookKey, entry: ActivationEntry) -> bool {
        match self.entries.entry(key) {
            dashmap::Entry::Occupied(_) => false,
            dashmap::Entry::Vacant(v) => {
                v.insert(Arc::new(entry));
                true
            },
        }
    }

    /// Look up an entry by key. Returns a cheap `Arc` clone so the
    /// transport can drop the DashMap guard before `await`-ing the
    /// handler.
    pub(crate) fn lookup(&self, key: &WebhookKey) -> Option<Arc<ActivationEntry>> {
        self.entries.get(key).map(|e| Arc::clone(&*e))
    }

    /// Remove an entry. Called by the transport on trigger
    /// deactivation.
    pub(crate) fn remove(&self, key: &WebhookKey) -> bool {
        self.entries.remove(key).is_some()
    }

    /// Replace all slug entries with a new set. Used by the admin
    /// reload endpoint (E3); programmatic activations are preserved.
    ///
    /// **Atomicity note (M3.3 / ADR-0049 § "Out of scope"):**
    /// The swap is performed by removing every slug entry and then
    /// inserting the new set into the shared `DashMap`. Concurrent
    /// readers between the two phases can observe a transient empty
    /// slug map and receive `404 Not Found`. The window is bounded
    /// by the size of the activation list (typically O(ms) for
    /// thousands of rows) and is acceptable for the M3.3 scope —
    /// admin reload is a low-frequency operator action, not a hot
    /// path.
    ///
    /// True atomic swap (no transient 404s under concurrent readers)
    /// requires moving slug entries behind an `ArcSwap<HashMap<...>>`
    /// or an `RwLock`-guarded map; it is tracked as a 1.0 follow-up
    /// alongside the broader concurrency hardening pass.
    pub(crate) fn replace_slug_entries(&self, new: Vec<(WebhookKey, ActivationEntry)>) {
        // Drop existing slug entries (programmatic activations stay
        // in place — they are owned by the typed runtime).
        self.entries
            .retain(|k, _| !matches!(k, WebhookKey::Slug(_)));
        for (key, entry) in new {
            self.entries.insert(key, Arc::new(entry));
        }
    }

    /// Number of entries with the given kind label, for telemetry
    /// gauges (M3.3 / `NEBULA_WEBHOOK_REGISTRATIONS`).
    #[must_use]
    #[allow(dead_code)] // wired into G2 metrics in a subsequent commit
    pub(crate) fn count_by_kind(&self, kind: &'static str) -> usize {
        self.entries
            .iter()
            .filter(|e| e.key().kind_label() == kind)
            .count()
    }

    /// Current number of active registrations. Used by transport
    /// observability accessors and tests.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use nebula_action::TriggerContext;
    use uuid::Uuid;

    use super::{
        super::key::{TriggerCoordinates, WebhookKey},
        *,
    };

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
        async fn start(&self, _ctx: &dyn TriggerContext) -> Result<(), nebula_action::ActionError> {
            Ok(())
        }
        async fn stop(&self, _ctx: &dyn TriggerContext) -> Result<(), nebula_action::ActionError> {
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
        let ctx = TriggerRuntimeContext::new(
            Arc::new(
                nebula_core::BaseContext::builder()
                    .cancellation(CancellationToken::new())
                    .build(),
            ),
            nebula_core::WorkflowId::new(),
            nebula_core::node_key!("test"),
        );
        ActivationEntry {
            handler,
            ctx,
            config: WebhookConfig::default(),
        }
    }

    #[test]
    fn insert_lookup_remove_roundtrip_programmatic() {
        let map = RoutingMap::new();
        let key = WebhookKey::programmatic(Uuid::new_v4(), "nonce1");
        assert!(map.insert(key.clone(), dummy_entry()));
        assert_eq!(map.len(), 1);
        assert!(map.lookup(&key).is_some());
        assert!(map.remove(&key));
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn insert_lookup_remove_roundtrip_slug() {
        let map = RoutingMap::new();
        let key = WebhookKey::slug(TriggerCoordinates::new("acme", "main", "github"));
        assert!(map.insert(key.clone(), dummy_entry()));
        assert!(map.lookup(&key).is_some());
        assert!(map.remove(&key));
    }

    #[test]
    fn insert_rejects_duplicate_key() {
        let map = RoutingMap::new();
        let key = WebhookKey::programmatic(Uuid::new_v4(), "n");
        assert!(map.insert(key.clone(), dummy_entry()));
        assert!(!map.insert(key, dummy_entry()));
    }

    #[test]
    fn replace_slug_entries_preserves_programmatic() {
        let map = RoutingMap::new();
        let prog = WebhookKey::programmatic(Uuid::new_v4(), "n");
        map.insert(prog.clone(), dummy_entry());

        let slug_a = WebhookKey::slug(TriggerCoordinates::new("a", "b", "c"));
        let slug_b = WebhookKey::slug(TriggerCoordinates::new("a", "b", "d"));
        map.insert(slug_a.clone(), dummy_entry());

        // Swap in just one slug entry — the original slug is gone,
        // the new one lives, programmatic survives.
        map.replace_slug_entries(vec![(slug_b.clone(), dummy_entry())]);

        assert!(map.lookup(&prog).is_some());
        assert!(map.lookup(&slug_a).is_none());
        assert!(map.lookup(&slug_b).is_some());
    }

    #[test]
    fn count_by_kind_separates_programmatic_and_slug() {
        let map = RoutingMap::new();
        map.insert(
            WebhookKey::programmatic(Uuid::new_v4(), "n1"),
            dummy_entry(),
        );
        map.insert(
            WebhookKey::slug(TriggerCoordinates::new("a", "b", "c")),
            dummy_entry(),
        );
        map.insert(
            WebhookKey::slug(TriggerCoordinates::new("a", "b", "d")),
            dummy_entry(),
        );
        assert_eq!(map.count_by_kind("programmatic"), 1);
        assert_eq!(map.count_by_kind("slug"), 2);
    }
}
