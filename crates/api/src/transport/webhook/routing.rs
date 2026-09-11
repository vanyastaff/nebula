//! Programmatic webhook routing map keyed on [`WebhookKey`].
//!
//! Runtime-minted `(uuid, nonce)` activations live in the same `DashMap`.
//! The transport's `dispatch_inner` performs a single lookup and runs the same
//! signature → replay → rate-limit → pre-handle → handle pipeline for every
//! registered activation.
//!
//! Slug-routed activations were retired in ADR-0096 commit 3; the map is
//! now programmatic-only.

use std::sync::Arc;

use dashmap::DashMap;
use nebula_action::{TriggerHandler, TriggerRuntimeContext, WebhookConfig};

use super::key::WebhookKey;

/// Single registered webhook activation.
///
/// Holds the handler pointer, a template [`TriggerRuntimeContext`]
/// that the transport clones on every request, and the
/// [`WebhookConfig`] enforced before dispatch. Cloning
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
            .field("handler_key", self.handler.metadata().base().key())
            .field("trigger_id", &self.ctx.trigger_id())
            .field("workflow_id", &self.ctx.scope().workflow_id)
            .finish_non_exhaustive()
    }
}

/// Thread-safe [`WebhookKey`] → activation lookup table.
#[derive(Debug, Default, Clone)]
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

    /// Current number of active registrations. Used by transport
    /// observability accessors and tests.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use nebula_action::{
        Action, ActionMetadataDraft, TriggerContext, TriggerEventOutcome, WebhookAction,
        WebhookRequest, WebhookResponse, WebhookTriggerAdapter,
    };
    use nebula_core::Dependencies;
    use uuid::Uuid;

    use super::{super::key::WebhookKey, *};

    struct NoopWebhook;

    impl Action for NoopWebhook {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> ActionMetadataDraft {
            ActionMetadataDraft::new(
                nebula_core::action_key!("test.routing.noop"),
                nebula_action::metadata_name!("Noop"),
                "routing map unit test",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static DEPENDENCIES: std::sync::OnceLock<Dependencies> = std::sync::OnceLock::new();
            DEPENDENCIES.get_or_init(Dependencies::new)
        }
    }

    impl WebhookAction for NoopWebhook {
        type State = ();

        async fn on_activate(
            &self,
            _ctx: &(impl TriggerContext + ?Sized),
        ) -> Result<(), nebula_action::ActionError> {
            Ok(())
        }

        async fn handle_request(
            &self,
            _request: &WebhookRequest,
            _state: &(),
            _ctx: &(impl TriggerContext + ?Sized),
        ) -> Result<WebhookResponse, nebula_action::ActionError> {
            Ok(WebhookResponse::accept(TriggerEventOutcome::skip()))
        }
    }

    fn dummy_entry() -> ActivationEntry {
        use tokio_util::sync::CancellationToken;
        let handler: Arc<dyn TriggerHandler> = Arc::new(
            WebhookTriggerAdapter::new(NoopWebhook).expect("valid test catalog definition"),
        );
        let ctx = TriggerRuntimeContext::new(
            Arc::new(
                nebula_core::BaseContext::builder(nebula_core::scope::Scope::default())
                    .principal(nebula_core::scope::Principal::System)
                    .cancellation(CancellationToken::new())
                    .build()
                    .expect("scope + principal must produce a valid BaseContext"),
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
    fn insert_rejects_duplicate_key() {
        let map = RoutingMap::new();
        let key = WebhookKey::programmatic(Uuid::new_v4(), "n");
        assert!(map.insert(key.clone(), dummy_entry()));
        assert!(!map.insert(key, dummy_entry()));
    }

    #[test]
    fn len_reflects_insertions() {
        let map = RoutingMap::new();
        map.insert(
            WebhookKey::programmatic(Uuid::new_v4(), "n1"),
            dummy_entry(),
        );
        map.insert(
            WebhookKey::programmatic(Uuid::new_v4(), "n2"),
            dummy_entry(),
        );
        assert_eq!(map.len(), 2);
    }
}
